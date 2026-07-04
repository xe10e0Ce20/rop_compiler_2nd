use std::collections::HashMap;
use miette::{miette, Result};
use crate::ast::*;

pub struct Compiler {
    pub base_address: u16,
    pub current_offset: u16,
    pub output: Vec<u8>,
    pub macro_registry: HashMap<String, MacroDef>,
    pub symbol_table: HashMap<String, u16>, 
    pub raw_symbol_table: HashMap<String, u16>,
}

impl Compiler {
    pub fn new() -> Self {
        Self {
            base_address: 0,
            current_offset: 0,
            output: Vec::new(),
            macro_registry: HashMap::new(),
            symbol_table: HashMap::new(),
            raw_symbol_table: HashMap::new(),
        }
    }

    pub fn register_macro(&mut self, def: MacroDef) {
        self.macro_registry.insert(def.name.clone(), def);
    }

    pub fn compile(&mut self, ast: &RopFile) -> Result<u16> {
        for item in &ast.items {
            if let TopLevelItem::MacroDef(m) = item {
                self.register_macro(m.clone());
            }
        }

        self.pre_scan_symbols(ast)?;

        self.base_address = 0;
        self.current_offset = 0;
        self.output.clear();

        for item in &ast.items {
            match item {
                TopLevelItem::Instruction(inst) => self.process_instruction(inst)?,
                TopLevelItem::Block(block) => self.process_block(block)?,
                _ => {}
            }
        }
        Ok(self.base_address)
    }

    fn pre_scan_symbols(&mut self, ast: &RopFile) -> Result<()> {
        let mut sim_offset = 0;

        for item in &ast.items {
            println!("调试：扫描到顶层项目 -> {:?}", item);
            match item {
                TopLevelItem::Instruction(Instruction::Offset(new_offset)) => {
                    sim_offset = *new_offset;
                }
                TopLevelItem::Block(block) => {
                    self.symbol_table.insert(block.name.clone(), sim_offset);
                    
                    let mut inner_offset = sim_offset;
                    for node in &block.contents {
                        match node {
                            Node::Label(name) => {
                                // 关键：标签在块内的偏移就是当前 inner_offset
                                self.symbol_table.insert(name.clone(), inner_offset);
                                println!("注册块内标签: {} -> 0x{:04X}", name, inner_offset);
                            }
                            _ => {
                                // 只有产生字节码的节点才增加偏移
                                inner_offset += self.calculate_node_size(node, &None, &HashMap::new())?;
                            }
                        }
                    }
                    sim_offset = inner_offset;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn calculate_block_size(&self, block: &Block, trailing_body: &Option<Vec<Node>>, arg_env: &HashMap<String, u16>) -> Result<u16> {
        let mut size = 0;
        for node in &block.contents {
            size += self.calculate_node_size(node, trailing_body, arg_env)?;
        }
        Ok(size)
    }

    fn calculate_node_size(&self, node: &Node, trailing_body: &Option<Vec<Node>>, arg_env: &HashMap<String, u16>) -> Result<u16> {
        match node {
            Node::Label(_) => Ok(0),
            Node::Instruction(Instruction::Offset(_)) => Ok(0),
            Node::Value(expr) => {
                if expr.raw_tokens.len() == 1 {
                    let t = &expr.raw_tokens[0];
                    if !(t.starts_with("0x")) && t.len() == 2 && t.chars().all(|c| c.is_ascii_hexdigit()) {
                        return Ok(1);
                    }
                }
                Ok(2)
            }
            Node::Yield => {
                let mut size = 0;
                if let Some(body_nodes) = trailing_body {
                    for inner_node in body_nodes {
                        size += self.calculate_node_size(inner_node, &None, arg_env)?;
                    }
                }
                Ok(size)
            }
            Node::MacroCall { name, args, body } => {
                let macro_def = self.macro_registry.get(name).ok_or_else(|| miette!("未定义的宏 '{}'", name))?;
                let mut next_env = arg_env.clone();
                for (i, param_name) in macro_def.params.iter().enumerate() {
                    if let Some(_arg_expr) = args.get(i) { // 【修复点：加下划线消除警告】
                        next_env.insert(param_name.clone(), 0);
                    }
                }
                let mut size = 0;
                for macro_node in &macro_def.body {
                    size += self.calculate_node_size(macro_node, body, &next_env)?;
                }
                Ok(size)
            }
        }
    }

    fn process_instruction(&mut self, inst: &Instruction) -> Result<()> {
        match inst {
            Instruction::Offset(new_offset) => {
                let new_offset = *new_offset;
                if self.base_address == 0 && self.output.is_empty() {
                    self.base_address = new_offset;
                    self.current_offset = new_offset;
                } else {
                    if new_offset < self.current_offset {
                        return Err(miette!("offset 试图回退地址"));
                    }
                    let gap = (new_offset - self.current_offset) as usize;
                    self.output.resize(self.output.len() + gap, 0x00);
                    self.current_offset = new_offset;
                }
            }
        }
        Ok(())
    }

    fn process_block(&mut self, block: &Block) -> Result<()> {
        for node in &block.contents {
            self.process_node(node, &None, &HashMap::new())?;
        }
        Ok(())
    }

    fn parse_single_token(&self, t: &str, arg_env: &HashMap<String, u16>) -> Result<(u16, bool)> {
        if t.starts_with("0x") {
            let val = u16::from_str_radix(t.trim_start_matches("0x"), 16)
                .map_err(|e| miette!("无效十六进制数 '{}': {}", t, e))?;
            Ok((val, false))
        } else if t.len() == 2 && t.chars().all(|c| c.is_ascii_hexdigit()) {
            let val = u16::from_str_radix(t, 16).unwrap();
            Ok((val, true))
        } else if let Some(&val) = arg_env.get(t) {
            Ok((val, false))
        } 
        // 【新增】处理 $name 语法
        else if t.starts_with('$') {
            let name = &t[1..];
            self.raw_symbol_table.get(name)
                .copied()
                .map(|v| (v, false))
                .ok_or_else(|| miette!("未定义的原始地址标签 '{}'", t))
        } 
        // 处理普通 name 语法
        else if let Some(&val) = self.symbol_table.get(t) {
            Ok((val, false))
        } else {
            Err(miette!("未定义的标识符 '{}'", t))
        }
    }

    fn evaluate_expr(&self, expr: &Expr, arg_env: &HashMap<String, u16>) -> Result<(u16, bool)> {
        if expr.raw_tokens.is_empty() { return Ok((0, false)); }
        if expr.raw_tokens.len() == 1 {
            return self.parse_single_token(&expr.raw_tokens[0], arg_env);
        }

        let (mut total, _) = self.parse_single_token(&expr.raw_tokens[0], arg_env)?;
        let mut i = 1;
        while i < expr.raw_tokens.len() {
            let op = &expr.raw_tokens[i];
            let (next_val, _) = self.parse_single_token(&expr.raw_tokens[i+1], arg_env)?;
            match op.as_str() {
                "+" => total = total.wrapping_add(next_val),
                "-" => total = total.wrapping_sub(next_val),
                _ => return Err(miette!("不支持的运算符")),
            }
            i += 2;
        }
        Ok((total, false))
    }

    fn process_node(&mut self, node: &Node, trailing_body: &Option<Vec<Node>>, arg_env: &HashMap<String, u16>) -> Result<()> {
        match node {
            Node::Label(_) => Ok(()),

            // 分支 1: 使用 ? 解包 Result，得到 ()
            Node::Instruction(inst) => self.process_instruction(inst),

            // 分支 2: 执行完逻辑后，最后补上 Ok(())
            Node::Value(expr) => {
                let (val, is_single_byte) = self.evaluate_expr(expr, arg_env)?;
                if is_single_byte && expr.endian.is_none() {
                    self.output.push(val as u8);
                    self.current_offset += 1;
                } else {
                    let is_be = expr.endian.as_deref() == Some("be");
                    let bytes = if is_be { val.to_be_bytes() } else { val.to_le_bytes() };
                    for b in &bytes {
                        self.output.push(*b);
                        self.current_offset += 1;
                    }
                }
                Ok(())
            }

            // 分支 3: 执行完循环后，补上 Ok(())
            Node::Yield => {
                if let Some(body_nodes) = trailing_body {
                    for inner_node in body_nodes {
                        self.process_node(inner_node, &None, arg_env)?;
                    }
                }
                Ok(())
            }

            // 分支 4: 循环调用后，补上 Ok(())
            Node::MacroCall { name, args, body } => {
                let macro_def = self.macro_registry.get(name).cloned().ok_or_else(|| miette!("未定义的宏"))?;
                let mut next_env = arg_env.clone();
                for (i, param_name) in macro_def.params.iter().enumerate() {
                    if let Some(arg_expr) = args.get(i) {
                        let (resolved_val, _) = self.evaluate_expr(arg_expr, arg_env)?;
                        next_env.insert(param_name.clone(), resolved_val);
                    }
                }
                for macro_node in &macro_def.body {
                    self.process_node(macro_node, body, &next_env)?;
                }
                Ok(())
            }
        }
    }
}