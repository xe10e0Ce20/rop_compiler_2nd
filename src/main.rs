mod ast;
mod parser;
mod compiler;

use miette::Result;

fn main() -> Result<()> {
    let source_code = r#"
        macro call_gadget(target_addr) {
            _label:
            A8 21              
            _label.le     // 宏内部直接把传进来的标签地址按小端吐出
        }

        @offset(0xd710)
        block main {
            00 11 22 33        
                        
            gadget_pop_rdi:
            
            // 1. 大统一：直接把地址标签作为宏参数传进去！
            call_gadget(gadget_pop_rdi)

            // 2. 标签还可以直接参与编译期运算！
            gadget_shell + 0x0004.be 
            
            44 55 66 77

            5F C3              // 占 2 字节，地址应当是 0xD710 + 4(main前4字节) + 2(A8 21) + 2(地址) + 2(算术) + 4(main后4字节) = 0xD722

            gadget_shell:

            AA BB CC DD EE FF  // 占 6 字节，地址在 gadget_pop_rdi 后面，即 0xD724
        }
    "#;

    println!("1. 解析 ROP 源代码...");
    let ast_tree = parser::parse_to_ast(source_code)?;

    println!("\n2. 启动双遍扫描标签编译器...");
    let mut compiler = compiler::Compiler::new();
    compiler.compile(&ast_tree)?;

    println!("\n>>> 编译期符号表(Labels)状态:");
    for (name, addr) in &compiler.symbol_table {
        println!("标签 '{}' -> 绝对内存地址: 0x{:04X}", name, addr);
    }

    println!("\n3. 编译完成！栈内存拓扑图视图:");
    println!("--------------------------------------------------");
    println!("地址\t| 字节码内容");
    println!("--------------------------------------------------");
    
    let mut print_offset = 0xd710;
    for chunk in compiler.output.chunks(4) {
        print!("0x{:04X}\t| ", print_offset);
        for byte in chunk {
            print!("{:02X} ", byte);
        }
        println!();
        print_offset += chunk.len() as u16;
    }
    println!("--------------------------------------------------");

    Ok(())
}