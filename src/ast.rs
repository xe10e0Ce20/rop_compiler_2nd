// 在 ast.rs 顶部添加导入
use std::ops::Range;

/// 带有位置跨度的包装器
#[derive(Debug, Clone)]
pub struct Spanned<T> {
    pub node: T,
    /// 字符偏离量区间：[start_byte, end_byte]，专门用于 miette 错误高亮或前端计算行/列
    pub span: Range<usize>,
}

/// 整个 ROP 文件的顶层 AST
#[derive(Debug, Clone)]
pub struct RopFile {
    pub items: Vec<TopLevelItem>,
}

#[derive(Debug, Clone)]
pub enum TopLevelItem {
    Import(String),
    MacroDef(MacroDef),
    Instruction(Instruction, Range<usize>),
    Block(Block),
}

#[derive(Debug, Clone)]
pub struct MacroDef {
    pub name: String,
    pub params: Vec<String>,
    pub body: Vec<Spanned<Node>>, // 升级为带位置标记的节点
}

#[derive(Debug, Clone)]
pub enum Instruction {
    Offset(u16),
    SetFiller(char),
}

#[derive(Debug, Clone)]
pub struct Block {
    pub name: String,
    pub contents: Vec<Spanned<Node>>, // 升级为带位置标记的节点
}

#[derive(Debug, Clone)]
pub enum Node {
    Instruction(Instruction),
    Yield,
    Value(Spanned<Expr>), // 升级表达式位置
    MacroCall {
        name: String,
        args: Vec<Spanned<Expr>>, // 升级参数表达式位置
        body: Option<Vec<Spanned<Node>>>, // 升级宏内容位置
    },
    Label(String),
}

// ----------------- 以下保持原有逻辑不变 -----------------
#[derive(Debug, Clone, Copy)]
pub struct RopValue {
    pub val: u64,
    pub len: usize,
}

impl RopValue {
    pub fn new(val: u64, len: usize) -> Self { Self { val, len } }
    pub fn math_op(&self, other: &Self, op: &str) -> Self {
        let new_len = self.len.max(other.len);
        let res = match op {
            "+" => self.val.wrapping_add(other.val),
            "-" => self.val.wrapping_sub(other.val),
            _ => 0,
        };
        RopValue::new(res, new_len)
    }
    pub fn concat(&self, other: &Self) -> Self {
        let new_val = (self.val << (other.len * 8)) | other.val;
        RopValue::new(new_val, self.len + other.len)
    }
    pub fn reverse_rop_bytes(val: u64, len: usize) -> u64 {
        let bytes = val.to_be_bytes();
        let mut target_bytes = bytes[8 - len..].to_vec();
        target_bytes.reverse();
        let mut new_val = 0u64;
        for (i, &b) in target_bytes.iter().enumerate() {
            new_val |= (b as u64) << ((len - 1 - i) * 8);
        }
        new_val
    }
}

#[derive(Debug, Clone)]
pub enum ExprToken {
    Raw(String),
    Op(String),
    Bracket(Expr), // 这里的 Expr 被解析函数递归调用
    Group(Expr),
}

#[derive(Debug, Clone)]
pub struct Expr {
    pub tokens: Vec<ExprToken>,
}