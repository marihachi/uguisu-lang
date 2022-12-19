use std::mem;

mod parser;
mod ast;
mod inst;
mod builtin;

pub fn run(code: &str) -> Result<(), String> {
    println!("[Info] compiling ...");
    let ast = parser::parse(code)
        .map_err(|e| format!("Compile Error: {}", e.message))?;

    let module = inst::emit_module(ast)
        .map_err(|e| format!("Compile Error: {}", e.message))?;

    let func = match module.funcs.iter().find(|x| x.name == "main") {
        Some(func) => func,
        None => return Err("Compile Error: function 'main' not found".to_string()),
    };

    println!("[Info] running ...");
    let func = unsafe { mem::transmute::<*const u8, fn()>(func.ptr) };
    func();

    Ok(())
}

#[cfg(test)]
mod test {
    use crate::*;

    fn run_test(code: &str) {
        match run(code) {
            Err(e) => {
                println!("{}", e);
                panic!();
            },
            _ => {},
        }
    }

    #[test]
    fn test_empty_return() {
        run_test("
            fn main() {
                return;
            }
        ");
    }

    #[test]
    fn text_basic() {
        run_test("
            external fn print_num(value: number);
            fn add(x: number, y: number): number {
                return x + y;
            }
            fn main() {
                print_num(add(1, 2) * 3);
            }
        ");
    }
}
