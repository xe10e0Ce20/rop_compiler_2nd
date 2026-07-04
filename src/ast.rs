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

#[derive(Debug, Clone)]
pub struct Expr {
    pub raw_tokens: Vec<String>,
    pub endian: Option<String>,
}