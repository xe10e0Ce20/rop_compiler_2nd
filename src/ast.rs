use std::ops::Range;

/// 参数类型声明，例如 4b 代表 4 字节
#[derive(Debug, Clone)]
pub struct TypeSpec {
    pub byte_len: usize,
}

/// 宏参数定义
#[derive(Debug, Clone)]
pub struct ParamDef {
    pub name: String,
    pub type_spec: Option<TypeSpec>,
    pub default: Option<Expr>,   // 默认值表达式（未求值）
}

#[derive(Debug, Clone)]
pub struct Spanned<T> {
    pub node: T,
    pub span: Range<usize>,
}

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
    pub params: Vec<ParamDef>,          // 改用 ParamDef 列表
    pub body: Vec<Spanned<Node>>,
}

#[derive(Debug, Clone)]
pub enum Instruction {
    Offset(u16),
    SetFiller(char),
}

#[derive(Debug, Clone)]
pub struct Block {
    pub name: String,
    pub contents: Vec<Spanned<Node>>,
}

#[derive(Debug, Clone)]
pub enum Node {
    Instruction(Instruction),
    Yield,
    Value(Spanned<Expr>),
    MacroCall {
        name: String,
        args: Vec<Spanned<Expr>>,
        body: Option<Vec<Spanned<Node>>>,
    },
    Label(String),
}

// RopValue 及其方法保持不变...
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
    Bracket(Expr),
    Group(Expr),
}

#[derive(Debug, Clone)]
pub struct Expr {
    pub tokens: Vec<ExprToken>,
}