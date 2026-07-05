#[derive(Debug, Clone)]
pub struct RopFile {
    pub items: Vec<TopLevelItem>,
}

#[derive(Debug, Clone)]
pub enum TopLevelItem {
    Import(String),
    MacroDef(MacroDef),
    Instruction(Instruction),
    Block(Block),
}

#[derive(Debug, Clone)]
pub struct MacroDef {
    pub name: String,
    pub params: Vec<String>,
    pub body: Vec<Node>,
}

#[derive(Debug, Clone)]
pub enum Instruction {
    Offset(u16),
}

#[derive(Debug, Clone)]
pub struct Block {
    pub name: String,
    pub contents: Vec<Node>,
}

#[derive(Debug, Clone)]
pub enum Node {
    Instruction(Instruction),
    Yield,
    Value(Expr), // 裸字节码、算术式全都在这里
    MacroCall {
        name: String,
        args: Vec<Expr>,
        body: Option<Vec<Node>>,
    },
    Label(String),
}

#[derive(Debug, Clone, Copy)]
pub struct RopValue {
    pub val: u64,
    pub len: usize,
}

impl RopValue {
    pub fn new(val: u64, len: usize) -> Self { Self { val, len } }
    
    // 加减法：取两者长度的最大值
    pub fn math_op(&self, other: &Self, op: &str) -> Self {
        let new_len = self.len.max(other.len);
        let res = match op {
            "+" => self.val.wrapping_add(other.val),
            "-" => self.val.wrapping_sub(other.val),
            _ => 0,
        };
        RopValue::new(res, new_len)
    }

    // 拼接操作 (|)：直接叠加长度和字节
    pub fn concat(&self, other: &Self) -> Self {
        let new_val = (self.val << (other.len * 8)) | other.val;
        RopValue::new(new_val, self.len + other.len)
    }
}

#[derive(Debug, Clone)]
pub struct Expr {
    pub raw_tokens: Vec<String>, // 包含操作符和操作数
    pub se: bool,               // 新增：标识该表达式是否加了 .se
    pub sub_expr: Option<Box<Expr>>, // 新增：处理括号逻辑
}