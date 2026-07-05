// src/errors.rs
use miette::{Diagnostic, SourceSpan};
use thiserror::Error;

#[derive(Error, Debug, Diagnostic)]
pub enum RopError {
    /// 1. 专门用于劫持和优化 Pest 的原始语法错误
    #[error("语法解析失败：\n{message}")]
    #[diagnostic(code(rop::parser::syntax_error))]
    SyntaxError {
        message: String,
        #[label("错误发生在此处")]
        span: SourceSpan,
    },

    /// 2. 专门用于语义编译、表达式求值、未定义符号错误
    #[error("编译语义错误：{message}")]
    #[diagnostic(code(rop::compiler::semantic_error))]
    CompileError {
        message: String,
        #[label("错误节点")]
        span: SourceSpan,
    },
}