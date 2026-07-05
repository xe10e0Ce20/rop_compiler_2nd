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

// parser.rs -> build_instruction
fn build_instruction(pair: Pair<Rule>) -> Result<Instruction> {
    // 获取指令的内部部分 (offset_cmd 或 filler_cmd)
    let inner = pair.into_inner().next()
        .ok_or_else(|| miette!("指令内容为空"))?; 
        
    match inner.as_rule() {
        Rule::offset_cmd => {
            let hex_pair = inner.into_inner().next()
                .ok_or_else(|| miette!("offset 缺失参数"))?;
            let hex_str = hex_pair.as_str().trim_start_matches("0x");
            let val = u16::from_str_radix(hex_str, 16)
                .map_err(|e| miette!("非法偏移值: {}", e))?;
            Ok(Instruction::Offset(val))
        }
        Rule::filler_cmd => {
            let hex_pair = inner.into_inner().next()
                .ok_or_else(|| miette!("filler 缺失参数"))?;
            let c = hex_pair.as_str().chars().next()
                .ok_or_else(|| miette!("无效填充符"))?;
            Ok(Instruction::SetFiller(c))
        }
        _ => Err(miette!("未知指令: {:?}", inner.as_rule())),
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
            // identifier 规则在语法里是 label_def 的一部分
            let name = pair.as_str().trim_end_matches(':').trim().to_string();
            Ok(Node::Label(name))
        }
        
        Rule::value_expr => Ok(Node::Value(build_expr(pair))),
        
        Rule::macro_call => {
            let mut inner = pair.into_inner();
            let name = inner.next().unwrap().as_str().to_string();
            let mut args = Vec::new();
            let mut body = None;
            
            for item in inner {
                match item.as_rule() {
                    Rule::macro_arg => {
                        // 直接构建 Expr，保存原始的 Token 序列，实现字面量粘贴
                        let val_expr_pair = item.into_inner().next().unwrap();
                        args.push(build_expr(val_expr_pair));
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
        
        _ => Err(miette!("未知节点类型: {:?}", pair.as_rule())),
    }
}