use pest::Parser;
use pest::iterators::Pair;
use miette::Result;
use crate::ast::*;
use crate::errors::RopError;
use std::ops::Range;

#[derive(pest_derive::Parser)]
#[grammar = "syntax.pest"]
pub struct RopParser;

pub fn parse_to_ast(source: &str) -> Result<RopFile, miette::Error> {
    let mut parsed = RopParser::parse(Rule::file, source).map_err(|e| {
        let (start, end) = match e.location {
            pest::error::InputLocation::Pos(pos) => (pos, pos + 1),
            pest::error::InputLocation::Span((start, end)) => (start, end),
        };
        RopError::SyntaxError {
            message: format!("{}", e),
            span: (start, end - start).into(),
        }
    })?;
    Ok(build_file(parsed.next().unwrap())?)
}

fn build_file(pair: Pair<Rule>) -> Result<RopFile> {
    let mut items = Vec::new();
    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::EOI => {}
            Rule::import_cmd => {
                let path = inner.into_inner().next().unwrap().as_str().to_string();
                items.push(TopLevelItem::Import(path));
            }
            Rule::macro_def => items.push(TopLevelItem::MacroDef(build_macro_def(inner)?)),
            Rule::instruction => {
                let span = inner.as_span().start()..inner.as_span().end();
                let inst = build_instruction(inner, &span)?;
                items.push(TopLevelItem::Instruction(inst, span));
            }
            Rule::block => items.push(TopLevelItem::Block(build_block(inner)?)),
            _ => {}
        }
    }
    Ok(RopFile { items })
}

fn parse_param(pair: Pair<Rule>) -> ParamDef {
    let mut inner = pair.into_inner();
    let name = inner.next().unwrap().as_str().to_string();
    let mut type_spec = None;
    let mut default = None;

    for part in inner {
        match part.as_rule() {
            Rule::type_spec => {
                let num_str = part.as_str().trim_end_matches('b');
                let byte_len: usize = num_str.parse().unwrap(); // 语法已保证纯数字
                type_spec = Some(TypeSpec { byte_len });
            }
            Rule::value_expr => {
                default = Some(build_expr(part));
            }
            _ => {}
        }
    }
    ParamDef { name, type_spec, default }
}

fn build_macro_def(pair: Pair<Rule>) -> Result<MacroDef> {
    let mut inner = pair.into_inner();
    let name = inner.next().unwrap().as_str().to_string();
    let mut params = Vec::new();
    let mut body = Vec::new();

    for item in inner {
        match item.as_rule() {
            Rule::param_list => {
                for p in item.into_inner() {
                    params.push(parse_param(p));
                }
            }
            Rule::value_expr | Rule::macro_call | Rule::instruction | Rule::yield_keyword | Rule::label_def => {
                let span_range = item.as_span().start()..item.as_span().end();
                body.push(Spanned {
                    node: build_node(item)?,
                    span: span_range,
                });
            }
            _ => {}
        }
    }
    Ok(MacroDef { name, params, body })
}

// build_instruction 增加 span 参数（已改）
fn build_instruction(pair: Pair<Rule>, span: &Range<usize>) -> Result<Instruction> {
    let inner = pair.into_inner().next()
        .ok_or_else(|| RopError::SyntaxError {
            message: "指令内容为空 / Instruction content is empty".to_string(),
            span: (span.start, span.len()).into(),
        })?;

    match inner.as_rule() {
        Rule::offset_cmd => {
            let hex_pair = inner.into_inner().next()
                .ok_or_else(|| RopError::SyntaxError {
                    message: "offset 指令缺失参数 / Missing argument for offset directive".to_string(),
                    span: (span.start, span.len()).into(),
                })?;
            let hex_str = hex_pair.as_str().trim_start_matches("0x");
            let val = u16::from_str_radix(hex_str, 16)
                .map_err(|e| RopError::SyntaxError {
                    message: format!("非法的偏移值 / Invalid offset value: {}", e),
                    span: (span.start, span.len()).into(),
                })?;
            Ok(Instruction::Offset(val))
        }
        Rule::filler_cmd => {
            let hex_pair = inner.into_inner().next()
                .ok_or_else(|| RopError::SyntaxError {
                    message: "filler 指令缺失参数 / Missing argument for filler directive".to_string(),
                    span: (span.start, span.len()).into(),
                })?;
            let c = hex_pair.as_str().chars().next()
                .ok_or_else(|| RopError::SyntaxError {
                    message: "无效的填充字符 / Invalid filler character".to_string(),
                    span: (span.start, span.len()).into(),
                })?;
            Ok(Instruction::SetFiller(c))
        }
        _ => Err(RopError::SyntaxError {
            message: format!("未知指令 / Unknown directive: {:?}", inner.as_rule()),
            span: (span.start, span.len()).into(),
        }.into()),
    }
}

fn build_block(pair: Pair<Rule>) -> Result<Block> {
    let mut inner = pair.into_inner();
    let name = inner.next().unwrap().as_str().to_string();
    let mut contents = Vec::new();
    for content_pair in inner {
        match content_pair.as_rule() {
            Rule::value_expr | Rule::macro_call | Rule::instruction | Rule::yield_keyword | Rule::label_def => {
                let span_range = content_pair.as_span().start()..content_pair.as_span().end();
                contents.push(Spanned {
                    node: build_node(content_pair)?,
                    span: span_range,
                });
            }
            _ => {}
        }
    }
    Ok(Block { name, contents })
}

fn build_expr(pair: Pair<Rule>) -> Expr {
    let mut tokens = Vec::new();
    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::expr_term => {
                let inner = child.into_inner().next().unwrap();
                match inner.as_rule() {
                    Rule::hex_number | Rule::byte_number | Rule::address_ref => {
                        tokens.push(ExprToken::Raw(inner.as_str().trim().to_string()));
                    }
                    Rule::expr_group => {
                        let val_expr = build_expr(inner.into_inner().next().unwrap());
                        tokens.push(ExprToken::Group(val_expr));
                    }
                    Rule::bracket_expr => {
                        let val_expr = build_expr(inner.into_inner().next().unwrap());
                        tokens.push(ExprToken::Bracket(val_expr));
                    }
                    _ => {}
                }
            }
            Rule::binary_op => {
                tokens.push(ExprToken::Op(child.as_str().trim().to_string()));
            }
            _ => {}
        }
    }
    Expr { tokens }
}

fn build_node(pair: Pair<Rule>) -> Result<Node> {
    match pair.as_rule() {
        Rule::yield_keyword => Ok(Node::Yield),
        Rule::label_def => {
            let name = pair.as_str().trim_end_matches(':').trim().to_string();
            Ok(Node::Label(name))
        }
        Rule::value_expr => {
            let span_range = pair.as_span().start()..pair.as_span().end();
            Ok(Node::Value(Spanned {
                node: build_expr(pair),
                span: span_range,
            }))
        }
        Rule::macro_call => {
            let mut inner = pair.into_inner();
            let name = inner.next().unwrap().as_str().to_string();
            let mut args = Vec::new();
            let mut body = None;

            for item in inner {
                match item.as_rule() {
                    Rule::macro_arg => {
                        let span_range = item.as_span().start()..item.as_span().end();
                        let val_expr_pair = item.into_inner().next().unwrap();
                        args.push(Spanned {
                            node: build_expr(val_expr_pair),
                            span: span_range,
                        });
                    }
                    Rule::macro_body => {
                        let mut b_vec = Vec::new();
                        for inner_node in item.into_inner() {
                            let node_span = inner_node.as_span().start()..inner_node.as_span().end();
                            b_vec.push(Spanned {
                                node: build_node(inner_node)?,
                                span: node_span,
                            });
                        }
                        body = Some(b_vec);
                    }
                    _ => {}
                }
            }
            Ok(Node::MacroCall { name, args, body })
        }
        Rule::instruction => {
            let span = pair.as_span().start()..pair.as_span().end();
            Ok(Node::Instruction(build_instruction(pair, &span)?))
        }
        _ => Err(miette::miette!("未知节点类型 / Unknown node type: {:?}", pair.as_rule())),
    }
}