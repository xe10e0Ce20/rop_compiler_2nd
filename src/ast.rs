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
    pub default: Option<Expr>,
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
    pub params: Vec<ParamDef>,
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

/// 任意长度字节值（大端字节序）
#[derive(Debug, Clone)]
pub struct RopValue {
    pub val: Vec<u8>,
    pub len: usize,      // 始终等于 val.len()
}

impl RopValue {
    /// 从 u64 和指定长度创建（用于地址、符号等，产生大端字节）
    pub fn from_u64(val: u64, len: usize) -> Self {
        let bytes = &val.to_be_bytes()[8 - len..];
        Self {
            val: bytes.to_vec(),
            len,
        }
    }

    /// 从字节切片直接创建
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        let len = bytes.len();
        Self { val: bytes, len }
    }

    /// 占位符构造（dry run 用）
    pub fn placeholder(len: usize) -> Self {
        Self {
            val: vec![0; len],
            len,
        }
    }

    /// 数学运算（+、-），仅当两个值长度相同且 ≤8 时有效，结果长度取两者中较大者
    pub fn math_op(&self, other: &Self, op: &str) -> Result<Self, String> {
        if self.len > 8 || other.len > 8 {
            return Err("数学运算仅支持 ≤8 字节的数值".into());
        }
        let a = self.to_u64();
        let b = other.to_u64();
        let res = match op {
            "+" => a.wrapping_add(b),
            "-" => a.wrapping_sub(b),
            _ => return Err(format!("不支持的操作符: {}", op)),
        };
        Ok(RopValue::from_u64(res, self.len.max(other.len)))
    }

    /// 拼接（| 操作符），直接合并两个字节序列
    pub fn concat(&self, other: &Self) -> Self {
        let mut new_bytes = self.val.clone();
        new_bytes.extend_from_slice(&other.val);
        RopValue::from_bytes(new_bytes)
    }

    /// 反序整个字节序列（用于 [ ... ]）
    pub fn reverse_bytes(&self) -> Self {
        let mut bytes = self.val.clone();
        bytes.reverse();
        RopValue::from_bytes(bytes)
    }

    /// 转为 u64（用于需要数值的场景，如地址计算、数学运算）
    pub fn to_u64(&self) -> u64 {
        let mut buf = [0u8; 8];
        let start = 8 - self.len;
        buf[start..].copy_from_slice(&self.val);
        u64::from_be_bytes(buf)
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