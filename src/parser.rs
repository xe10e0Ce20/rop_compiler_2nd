use pest::Parser;
use pest::iterators::Pair;
use miette::{miette, Result};
use crate::ast::*;

#[derive(pest_derive::Parser)]
#[grammar = "syntax.pest"]
pub struct RopParser;

pub fn parse_to_ast(source: &str) -> Result<RopFile> {
    let mut parsed = RopParser::parse(Rule::file, source)
        .map_err(|e| miette!("语法解析失败:\n{}", e))?;
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
            _ => {} // 优雅处理其他不需要的规则
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
            // 【修正点】：加上 Rule::label_def
            Rule::value_expr | Rule::macro_call | Rule::instruction | Rule::yield_keyword | Rule::label_def => {
                body.push(build_node(item)?);
            }
            _ => {}
        }
    }
    Ok(MacroDef { name, params, body })
}

fn build_instruction(pair: Pair<Rule>) -> Result<Instruction> {
    let inner = pair.into_inner().next().ok_or_else(|| miette!("空指令"))?;
    match inner.as_rule() {
        Rule::offset_cmd => {
            let hex_str = inner.into_inner().next().unwrap().as_str().trim_start_matches("0x");
            let val = u16::from_str_radix(hex_str, 16).map_err(|e| miette!("非法偏移值: {}", e))?;
            Ok(Instruction::Offset(val))
        }
        _ => Err(miette!("未知或不支持的指令")), // 【核心修复点】添加通配符兜底
    }
}

fn build_block(pair: Pair<Rule>) -> Result<Block> {
    let mut inner = pair.into_inner();
    let name = inner.next().unwrap().as_str().to_string();
    let mut contents = Vec::new();
    for content_pair in inner {
        match content_pair.as_rule() {
            // 【关键点】：在这里增加对 label_def 的匹配
            Rule::value_expr | Rule::macro_call | Rule::instruction | Rule::yield_keyword | Rule::label_def => {
                contents.push(build_node(content_pair)?);
            }
            _ => {} 
        }
    }
    Ok(Block { name, contents })
}

fn build_expr(pair: Pair<Rule>) -> Expr {
    let mut raw_tokens = Vec::new();
    let mut endian = None;

    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::expr_term | Rule::binary_op => {
                raw_tokens.push(child.as_str().trim().to_string());
            }
            Rule::endian => {
                endian = Some(child.as_str().to_string());
            }
            _ => {} // 兜底
        }
    }
    Expr { raw_tokens, endian }
}

fn build_node(pair: Pair<Rule>) -> Result<Node> {
    match pair.as_rule() {
        Rule::yield_keyword => Ok(Node::Yield),
        
        // 修正：这是处理 label_def 的正确方式
        Rule::label_def => {
            // 注意：如果语法中 identifier 是 label_def 的子项，这样取 name 是对的
            let name = pair.as_str().trim_end_matches(':').to_string();
            Ok(Node::Label(name))
        }
        
        Rule::value_expr => Ok(Node::Value(build_expr(pair))),
        
        Rule::macro_call => {
            // ... 原有的 macro_call 逻辑保持不变 ...
            let mut inner = pair.into_inner();
            let name = inner.next().unwrap().as_str().to_string();
            let mut args = Vec::new();
            let mut body = None;
            for item in inner {
                match item.as_rule() {
                    Rule::arg_list => {
                        for arg_pair in item.into_inner() {
                            args.push(build_expr(arg_pair));
                        }
                    }
                    Rule::macro_body => {
                        let mut b_vec = Vec::new();
                        for inner_node in item.into_inner() {
                            b_vec.push(build_node(inner_node)?);
                        }
                        body = Some(b_vec);
                    }
                    _ => {}
                }
            }
            Ok(Node::MacroCall { name, args, body })
        }
        
        Rule::instruction => Ok(Node::Instruction(build_instruction(pair)?)),
        
        _ => Err(miette!("未知节点: {:?}", pair.as_rule())),
    }
}