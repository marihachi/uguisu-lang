use crate::ast::{self, BinaryExpr, Operator, Literal};
use crate::builtin;
use crate::errors::CompileError;
use core::panic;
use cranelift_codegen::ir::{self, types, AbiParam, InstBuilder};
use cranelift_codegen::settings::{self, Configurable, Flags};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{self, default_libcall_names, FuncId, Linkage, Module};
use cranelift_native;
use std::collections::HashMap;
use target_lexicon::Architecture;

#[derive(Debug, Clone)]
struct FuncInfo {
    pub id: FuncId,
    pub params: Vec<FuncParamInfo>,
    pub ret_kind: Option<ValueKind>,
    pub is_external: bool,
}

#[derive(Debug, Clone)]
struct FuncParamInfo {
    pub name: String,
    pub value_kind: ValueKind,
}

#[derive(Debug, Clone)]
enum ValueKind {
    Number,
}

#[derive(Debug)]
struct FuncDeclInfo {
    pub name: String,
    pub id: FuncId,
    pub has_body: bool,
}

#[derive(Debug, Clone)]
pub struct CompiledModule {
    pub funcs: Vec<CompiledFunction>,
}

#[derive(Debug, Clone)]
pub struct CompiledFunction {
    pub name: String,
    pub ptr: *const u8,
}

pub fn emit_module(ast: Vec<ast::Statement>) -> Result<CompiledModule, CompileError> {
    let (mut module, mut ctx) = {
        let isa_builder = cranelift_native::builder().unwrap();
        let mut flag_builder = settings::builder();
        if let Err(e) = flag_builder.set("use_colocated_libcalls", "false") {
            panic!("Configuration error: {}", e.to_string());
        }
        // FIXME set back to true once the x64 backend supports it.
        let is_pic = isa_builder.triple().architecture != Architecture::X86_64;
        if let Err(e) = flag_builder.set("is_pic", if is_pic { "true" } else { "false" }) {
            panic!("Configuration error: {}", e.to_string());
        }
        let flags = Flags::new(flag_builder);
        let isa = match isa_builder.finish(flags) {
            Ok(isa) => isa,
            Err(e) => {
                panic!("Configuration error: {}", e.to_string());
            }
        };
        let mut module_builder = JITBuilder::with_isa(isa, default_libcall_names());
        let mut symbols: Vec<(&str, *const u8)> = Vec::new();
        symbols.push(("hello", builtin::hello as *const u8));
        symbols.push(("print_num", builtin::print_num as *const u8));
        // needs to declare the function signature for builtin functions using declare_func_internal
        // register builtin symbols.
        for symbol in symbols.iter() {
            module_builder.symbol(&symbol.0.to_string(), symbol.1);
        }
        let module = JITModule::new(module_builder);
        let ctx = module.make_context();
        (module, ctx)
    };
    let mut builder_ctx = FunctionBuilderContext::new();
    let mut func_table = HashMap::new();
    let mut func_decl_infos: Vec<FuncDeclInfo> = Vec::new();
    for statement in ast.iter() {
        match statement {
            ast::Statement::FunctionDeclaration(func_decl) => {
                let func_decl_info = emit_func_declaration(
                    &mut module,
                    &mut ctx,
                    &mut builder_ctx,
                    &func_decl,
                    &mut func_table,
                )?;
                func_decl_infos.push(func_decl_info);
            }
            _ => {
                println!("[Warn] variable declaration is unexpected in the global");
            }
        }
    }
    // finalize all functions
    module
        .finalize_definitions()
        .map_err(|_| CompileError::new("Failed to generate module."))?;
    let mut compiled_module = CompiledModule { funcs: Vec::new() };
    for func_decl_info in func_decl_infos {
        if func_decl_info.has_body {
            // get function ptr
            let func_ptr = module.get_finalized_function(func_decl_info.id);
            compiled_module.funcs.push(CompiledFunction {
                name: func_decl_info.name,
                ptr: func_ptr,
            });
        }
    }
    Ok(compiled_module)
}

fn emit_func_declaration(
    module: &mut JITModule,
    ctx: &mut cranelift_codegen::Context,
    builder_ctx: &mut FunctionBuilderContext,
    func_decl: &ast::FunctionDeclaration,
    func_table: &mut HashMap<String, FuncInfo>,
) -> Result<FuncDeclInfo, CompileError> {
    // TODO: To successfully resolve the identifier, the function declaration is made first.
    // declare function
    let mut params = Vec::new();
    for param in func_decl.params.iter() {
        let param_type = match &param.type_identifier {
            Some(type_name) => {
                // TODO: support other types
                if type_name != "number" {
                    return Err(CompileError::new("unknown type"));
                }
                ValueKind::Number
            }
            None => return Err(CompileError::new("Parameter type is not specified.")),
        };
        params.push(FuncParamInfo {
            name: param.identifier.clone(),
            value_kind: param_type,
        });
    }
    let ret_kind = match &func_decl.ret {
        Some(type_name) => {
            // TODO: support other types
            if type_name != "number" {
                return Err(CompileError::new("unknown type"));
            }
            Some(ValueKind::Number)
        }
        None => None,
    };
    let name = func_decl.identifier.clone();
    let is_external = func_decl.attributes.contains(&ast::FunctionAttribute::External);
    for param in params.iter() {
        match param.value_kind {
            ValueKind::Number => {
                ctx.func.signature.params.push(AbiParam::new(types::I32));
            }
        }
    }
    match ret_kind {
        Some(ValueKind::Number) => {
            ctx.func.signature.returns.push(AbiParam::new(types::I32));
        }
        None => {}
    }
    let linkage = if is_external {
        Linkage::Import
    } else {
        Linkage::Local
    };
    let func_id = match module.declare_function(&name, linkage, &ctx.func.signature) {
        Ok(id) => id,
        Err(_) => return Err(CompileError::new("Failed to declare a function.")),
    };
    println!("[Info] function '{}' is declared.", func_decl.identifier);
    // make ir signature
    let func_info = FuncInfo {
        id: func_id,
        params: params,
        ret_kind: ret_kind,
        is_external: is_external,
    };
    // register func table
    func_table.insert(func_decl.identifier.clone(), func_info.clone());

    let dec_info = if let Some(body) = &func_decl.body {
        let mut emitter = FunctionEmitter::new(module, ctx, builder_ctx, func_table);
        // emit the function
        emitter.emit_body(body, &func_info)?;
        // define the function
        if let Err(_) = module.define_function(func_info.id, ctx) {
            return Err(CompileError::new("failed to define a function."));
        };
        FuncDeclInfo {
            name: name,
            id: func_id,
            has_body: true,
        }
    } else {
        FuncDeclInfo {
            name: name,
            id: func_id,
            has_body: false,
        }
    };

    // clear the function context
    module.clear_context(ctx);
    Ok(dec_info)
}

struct FunctionEmitter<'a> {
    module: &'a mut JITModule,
    builder: FunctionBuilder<'a>,
    func_table: &'a HashMap<String, FuncInfo>,
    is_returned: bool,
}

impl<'a> FunctionEmitter<'a> {
    pub fn new(
        module: &'a mut JITModule,
        ctx: &'a mut cranelift_codegen::Context,
        builder_ctx: &'a mut FunctionBuilderContext,
        func_table: &'a HashMap<String, FuncInfo>,
    ) -> Self {
        let builder = FunctionBuilder::new(&mut ctx.func, builder_ctx);
        Self {
            module: module,
            builder: builder,
            func_table: func_table,
            is_returned: false,
        }
    }

    pub fn emit_body(
        &mut self,
        body: &Vec<ast::Statement>,
        func_info: &FuncInfo,
    ) -> Result<(), CompileError> {
        //self.builder.func.name = UserFuncName::user(0, func_info.id.as_u32());
        let block = self.builder.create_block();
        self.builder.switch_to_block(block);
        if func_info.params.len() > 0 {
            self.builder.append_block_params_for_function_params(block);
        }

        for statement in body.iter() {
            self.emit_statement(&func_info, block, statement)?;
        }
        // If there is no jump/return at the end of the block,
        // the emitter must implicitly emit a return instruction.
        if !self.is_returned {
            self.builder.ins().return_(&[]);
        }
        self.builder.seal_all_blocks();
        self.builder.finalize();

        Ok(())
    }

    fn emit_statement(
        &mut self,
        func: &FuncInfo,
        block: ir::Block,
        statement: &ast::Statement,
    ) -> Result<(), CompileError> {
        match statement {
            ast::Statement::ReturnStatement(Some(expr)) => {
                // When the return instruction is emitted, the block is filled.
                let value = match self.emit_expr(func, block, &expr)? {
                    Some(v) => v,
                    None => return Err(CompileError::new("value not found")),
                };
                self.builder.ins().return_(&[value]);
                self.is_returned = true;
            }
            ast::Statement::ReturnStatement(None) => {
                self.builder.ins().return_(&[]);
                self.is_returned = true;
            }
            ast::Statement::VariableDeclaration(statement) => {
                // TODO: use statement.identifier
                // TODO: use statement.attributes
                let value = match self.emit_expr(func, block, &statement.body)? {
                    Some(v) => v,
                    None => {
                        return Err(CompileError::new("The expression does not return a value."))
                    }
                };
                return Err(CompileError::new(
                    "variable declaration is not supported yet.",
                ));
            }
            ast::Statement::Assignment(statement) => {
                // TODO: use statement.identifier
                let value = match self.emit_expr(func, block, &statement.body)? {
                    Some(v) => v,
                    None => {
                        return Err(CompileError::new("The expression does not return a value."))
                    }
                };
                return Err(CompileError::new("assign statement is not supported yet."));
            }
            ast::Statement::FunctionDeclaration(_) => {
                return Err(CompileError::new("FuncDeclaration is unexpected"));
            }
            ast::Statement::ExprStatement(expr) => {
                self.emit_expr(func, block, expr)?;
            }
        }
        Ok(())
    }

    fn emit_expr(
        &mut self,
        func: &FuncInfo,
        block: ir::Block,
        expr: &ast::Expression,
    ) -> Result<Option<ir::Value>, CompileError> {
        match expr {
            ast::Expression::Literal(Literal::Number(value)) => self.emit_number(*value),
            ast::Expression::BinaryExpr(op) => self.emit_binary_op(func, block, op),
            ast::Expression::CallExpr(call_expr) => self.emit_call(func, block, call_expr),
            ast::Expression::Identifier(identifier) => {
                self.emit_identifier(func, block, identifier)
            }
        }
    }

    fn emit_number(&mut self, value: i32) -> Result<Option<ir::Value>, CompileError> {
        Ok(Some(
            self.builder.ins().iconst(types::I32, i64::from(value)),
        ))
    }

    fn emit_binary_op(
        &mut self,
        func: &FuncInfo,
        block: ir::Block,
        binary_expr: &BinaryExpr,
    ) -> Result<Option<ir::Value>, CompileError> {
        let left = match self.emit_expr(func, block, &binary_expr.left)? {
            Some(v) => v,
            None => return Err(CompileError::new("value not found")),
        };
        let right = match self.emit_expr(func, block, &binary_expr.right)? {
            Some(v) => v,
            None => return Err(CompileError::new("value not found")),
        };
        let value = match binary_expr.operator {
            Operator::Add => self.builder.ins().iadd(left, right),
            Operator::Sub => self.builder.ins().isub(left, right),
            Operator::Mult => self.builder.ins().imul(left, right),
            Operator::Div => self.builder.ins().udiv(left, right),
        };
        Ok(Some(value))
    }

    fn emit_call(
        &mut self,
        func: &FuncInfo,
        block: ir::Block,
        call_expr: &ast::CallExpr,
    ) -> Result<Option<ir::Value>, CompileError> {
        let callee_name = match call_expr.callee.as_ref() {
            ast::Expression::Identifier(name) => name,
            _ => return Err(CompileError::new("invalid callee kind")),
        };
        let callee_func = match self.func_table.get(callee_name) {
            Some(info) => info,
            None => {
                let message = format!("unknown function '{}' is called.", callee_name);
                return Err(CompileError::new(&message));
            }
        };
        if callee_func.params.len() != call_expr.args.len() {
            return Err(CompileError::new("parameter count is incorrect"));
        }
        let func_ref = self
            .module
            .declare_func_in_func(callee_func.id, self.builder.func);
        let mut param_values = Vec::new();
        for arg in call_expr.args.iter() {
            match self.emit_expr(func, block, arg)? {
                Some(v) => {
                    param_values.push(v);
                }
                None => return Err(CompileError::new("The expression does not return a value.")),
            }
        }
        let call = self.builder.ins().call(func_ref, &param_values);
        let results = self.builder.inst_results(call);
        if results.len() > 0 {
            Ok(Some(results[0]))
        } else {
            Ok(None)
        }
    }

    fn emit_identifier(
        &mut self,
        func: &FuncInfo,
        block: ir::Block,
        identifier: &String,
    ) -> Result<Option<ir::Value>, CompileError> {
        match func.params.iter().position(|item| item.name == *identifier) {
            Some(index) => Ok(Some(self.builder.block_params(block)[index])),
            None => {
                Err(CompileError::new(
                    "Identifier of variables is not supported yet",
                )) // TODO
            }
        }
    }
}
