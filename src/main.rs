use miette::Result;

use rop_compiler::parser;
use rop_compiler::compiler;

fn main() -> Result<()> {
    let source_code = r#"
        def call_gadget(target_addr) {
            _label:
            A8 21     
            yield         
            [_label]    // 宏内部直接把传进来的标签地址按小端吐出
        }

        @offset(0xd710)
        block main {
        @filler(3)
            .. 11 22 33        
                        
            gadget_pop_rdi:
            
            // 1. 大统一：直接把地址标签作为宏参数传进去！
            call_gadget(( gadget_pop_rdi | 0x0001 )){
                a8 23 
            }

            // 2. 标签还可以直接参与编译期运算！
            gadget_shell | &gadget_shell
            

            5F C3              // 占 2 字节，地址应当是 0xD710 + 4(main前4字节) + 2(A8 21) + 2(地址) + 2(算术) + 4(main后4字节) = 0xD722

            gadget_shell:

            AA BB CC DD EE FF  // 占 6 字节，地址在 gadget_pop_rdi 后面，即 0xD724
        }
    "#;

    println!("1. 解析 ROP 源代码... / Parsing ROP source code...");
    let ast_tree = parser::parse_to_ast(source_code)?;

    println!("\n2. 解析标签... / Parsing labels...");
    let mut compiler = compiler::Compiler::new();
    compiler.compile(&ast_tree)?;

    println!("\n>>> 编译期符号表(Labels)状态 / Compile-time symbol table (Labels):");
    for (name, addr) in &compiler.symbol_table {
        println!(
            "标签 '{}' -> 绝对内存地址: 0x{:04X} / Label '{}' -> absolute address: 0x{:04X}",
            name, addr, name, addr
        );
    }

    println!("\n3. 编译完成 / Compilation complete! :");

    for (block_name, bytes) in &compiler.block_outputs {
        println!("\n[Block: {}]", block_name);
        println!("---------------------------------------------------------");
        println!("相对偏移| 字节码内容");
        println!("---------------------------------------------------------");

        let mut print_offset = 0;
        for chunk in bytes.chunks(16) {
            print!("+0x{:04X}\t| ", print_offset);
            for byte in chunk {
                print!("{:02X} ", byte);
            }
            println!();
            print_offset += chunk.len() as u16;
        }
        println!("---------------------------------------------------------");
    }

    let macros_for_frontend = compiler.get_autocomplete_metadata();
    println!(
        "\n[前端元数据调试] 可用的宏列表 / [Frontend metadata] available macros: {:?}",
        macros_for_frontend
    );

    Ok(())
}