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
    SetFiller(char),
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

    // 这是一个通用的辅助方法，你可以放在 Compiler 或 RopValue 中
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

// 新增 Token 枚举
#[derive(Debug, Clone)]
pub enum ExprToken {
    Raw(String),           // 纯变量/数字："0x0004", "A8", "&gadget"
    Op(String),            // 操作符："+", "-", "|"
    Bracket(Expr),         // 方括号：[ expr ]
    Group(Expr),           // 圆括号：( expr )
}

// 改造 Expr
#[derive(Debug, Clone)]
pub struct Expr {
    pub tokens: Vec<ExprToken>, 
}