/// 整个 ROP 文件的顶层 AST
#[derive(Debug, Clone)]
pub struct RopFile {
    pub items: Vec<TopLevelItem>,
}

/// 顶层条目：导入、宏定义、指令或代码块
#[derive(Debug, Clone)]
pub enum TopLevelItem {
    Import(String),
    MacroDef(MacroDef),
    Instruction(Instruction),
    Block(Block),
}

/// 宏定义：名称、参数列表、函数体
#[derive(Debug, Clone)]
pub struct MacroDef {
    pub name: String,
    pub params: Vec<String>,
    pub body: Vec<Node>,
}

/// 伪指令：设置偏移或填充字节
#[derive(Debug, Clone)]
pub enum Instruction {
    Offset(u16),
    SetFiller(char),
}

/// 命名代码块
#[derive(Debug, Clone)]
pub struct Block {
    pub name: String,
    pub contents: Vec<Node>,
}

/// 块/宏体内的节点
#[derive(Debug, Clone)]
pub enum Node {
    Instruction(Instruction),
    Yield,
    Value(Expr),
    MacroCall {
        name: String,
        args: Vec<Expr>,
        body: Option<Vec<Node>>,
    },
    Label(String),
}

/// 编译后的数值，含位宽
#[derive(Debug, Clone, Copy)]
pub struct RopValue {
    pub val: u64,
    pub len: usize,
}

impl RopValue {
    pub fn new(val: u64, len: usize) -> Self {
        Self { val, len }
    }

    /// 加法 / 减法：结果长度为两者较大者
    pub fn math_op(&self, other: &Self, op: &str) -> Self {
        let new_len = self.len.max(other.len);
        let res = match op {
            "+" => self.val.wrapping_add(other.val),
            "-" => self.val.wrapping_sub(other.val),
            _ => 0,
        };
        RopValue::new(res, new_len)
    }

    /// 拼接 (|) ：低位在前，高位在后
    pub fn concat(&self, other: &Self) -> Self {
        let new_val = (self.val << (other.len * 8)) | other.val;
        RopValue::new(new_val, self.len + other.len)
    }

    /// 反转字节序（用于小端输出）
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

/// 表达式中的词法单元
#[derive(Debug, Clone)]
pub enum ExprToken {
    Raw(String),           // 原始符号：数字、标识符
    Op(String),            // 运算符 + - |
    Bracket(Expr),         // [ ... ]
    Group(Expr),           // ( ... )
}

/// 表达式：由 Token 序列组成
#[derive(Debug, Clone)]
pub struct Expr {
    pub tokens: Vec<ExprToken>,
}