// src/parser.rs
use pest::Parser;
use pest::iterators::Pair;
use miette::Result;
use crate::ast::*;
use crate::errors::RopError;

#[derive(pest_derive::Parser)]
#[grammar = "syntax.pest"]
pub struct RopParser;

pub fn parse_to_ast(source: &str) -> Result<RopFile, miette::Error> {
    let mut parsed = RopParser::parse(Rule::file, source).map_err(|e| {
        // 精准提取 Pest 生成的具体位置偏离量信息，转换为 Miette 强化的跨度
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
            Rule::instruction => items.push(TopLevelItem::Instruction(build_instruction(inner)?)),
            Rule::block => items.push(TopLevelItem::Block(build_block(inner)?)),
            _ => {}
        }
    }
    Ok(RopFile { items })
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
                    params.push(p.as_str().to_string());
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

fn build_instruction(pair: Pair<Rule>) -> Result<Instruction> {
    let inner = pair.into_inner().next()
        .ok_or_else(|| miette::miette!("指令内容为空 / Instruction content is empty"))?;

    match inner.as_rule() {
        Rule::offset_cmd => {
            let hex_pair = inner.into_inner().next()
                .ok_or_else(|| miette::miette!("offset 缺失参数 / Missing argument for offset"))?;
            let hex_str = hex_pair.as_str().trim_start_matches("0x");
            let val = u16::from_str_radix(hex_str, 16)
                .map_err(|e| miette::miette!("非法偏移值: {0}", e))?;
            Ok(Instruction::Offset(val))
        }
        Rule::filler_cmd => {
            let hex_pair = inner.into_inner().next()
                .ok_or_else(|| miette::miette!("filler 缺失参数 / Missing argument for filler"))?;
            let c = hex_pair.as_str().chars().next()
                .ok_or_else(|| miette::miette!("无效填充符"))?;
            Ok(Instruction::SetFiller(c))
        }
        _ => Err(miette::miette!("未知指令: {:?}", inner.as_rule())),
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
        Rule::instruction => Ok(Node::Instruction(build_instruction(pair)?)),
        _ => Err(miette::miette!("未知节点类型: {:?}", pair.as_rule())),
    }
}