use crate::context::ModuleContext;
use crate::mappers::{contracts, events, functions, structs, types};
use crate::names::{self, build_type_desc};
use crate::utils::ZeroSpanNode;
use fe_analyzer::namespace::items::{Item, ModuleId, TypeDef};
use fe_analyzer::namespace::types::{Array, Tuple, Type};
use fe_analyzer::AnalyzerDb;
use fe_parser::ast::{self, SmolStr};
use fe_parser::node::Node;

/// Lowers a module.
pub fn module(db: &dyn AnalyzerDb, module: ModuleId) -> ast::Module {
    let mut context = ModuleContext::new(db, module);

    let mut lowered_body = module
        .ast(db)
        .body
        .iter()
        .filter_map(|stmt| match stmt {
            ast::ModuleStmt::Pragma(_) | ast::ModuleStmt::Use(_) => Some(stmt.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();

    lowered_body.extend(module.all_items(db).iter().filter_map(|item| match item {
        Item::Type(typedef) => match typedef {
            TypeDef::Alias(id) => {
                let node = &id.data(db).ast;
                let name = node.kind.name.clone();
                Some(ast::ModuleStmt::TypeAlias(Node::new(
                    ast::TypeAlias {
                        name,
                        typ: types::type_desc(
                            &mut context,
                            node.kind.typ.clone(),
                            &id.typ(db).expect("type alias error"),
                        ),
                        pub_qual: node.kind.pub_qual,
                    },
                    id.span(db),
                )))
            }
            TypeDef::Struct(id) => Some(ast::ModuleStmt::Struct(structs::struct_def(
                &mut context,
                *id,
            ))),
            TypeDef::Contract(id) => Some(ast::ModuleStmt::Contract(contracts::contract_def(
                &mut context,
                *id,
            ))),
            TypeDef::Primitive(_) => unreachable!(),
        },
        Item::Function(id) => Some(ast::ModuleStmt::Function(functions::func_def(
            &mut context,
            *id,
        ))),

        Item::GenericType(_) => todo!("generic types can't be defined in fe yet"),
        Item::Event(id) => Some(ast::ModuleStmt::Event(events::event_def(&mut context, *id))),
        Item::BuiltinFunction(_) | Item::Intrinsic(_) => {
            unreachable!("special built-in stuff")
        }

        Item::Constant(constant) => Some(ast::ModuleStmt::Constant(constant.data(db).ast.clone())),

        Item::Ingot(_) => unreachable!("ingots cannot be defined in a module"),
        Item::Module(_) => unreachable!("modules cannot be defined in modules (at least not yet)"),
    }));

    let struct_defs_from_tuples = context
        .tuples
        .iter()
        .map(|typ| ast::ModuleStmt::Struct(build_tuple_struct(typ).into_node()))
        .collect::<Vec<ast::ModuleStmt>>();

    let func_defs_from_list_expr = context
        .list_expressions
        .iter()
        .map(|expr| ast::ModuleStmt::Function(list_expr_to_fn_def(expr).into_node()))
        .collect::<Vec<_>>();

    ast::Module {
        body: [
            struct_defs_from_tuples,
            func_defs_from_list_expr,
            lowered_body,
        ]
        .concat(),
    }
}

fn build_tuple_struct(tuple: &Tuple) -> ast::Struct {
    let fields = tuple
        .items
        .iter()
        .enumerate()
        .map(|(index, typ)| {
            build_struct_field(format!("item{}", index), build_type_desc(typ)).into_node()
        })
        .collect();

    ast::Struct {
        name: names::tuple_struct_name(tuple).into_node(),
        fields,
        functions: vec![],
        pub_qual: None,
    }
}

fn build_struct_field(name: String, type_desc: ast::TypeDesc) -> ast::Field {
    ast::Field {
        is_pub: true,
        is_const: false,
        name: SmolStr::new(name).into_node(),
        typ: type_desc.into_node(),
        value: None,
    }
}

fn list_expr_to_fn_def(array: &Array) -> ast::Function {
    // Built the AST nodes for the function arguments
    let args = (0..array.size)
        .map(|index| {
            ast::FunctionArg::Regular(ast::RegularFunctionArg {
                label: Some(SmolStr::new("_").into_node()),
                name: SmolStr::new(format!("val{}", index)).into_node(),
                typ: build_type_desc(&Type::Base(array.inner)).into_node(),
            })
            .into_node()
        })
        .collect::<Vec<_>>();

    // Build the AST node for the array declaration
    let var_decl_name = "generated_array";
    let var_decl = ast::FuncStmt::VarDecl {
        target: ast::VarDeclTarget::Name(var_decl_name.into()).into_node(),
        typ: build_type_desc(&Type::Array(array.clone())).into_node(),
        value: None,
    }
    .into_node();

    // Build the AST nodes for the individual assignments of array slots
    let assignments = (0..array.size)
        .map(|index| {
            ast::FuncStmt::Assign {
                target: ast::Expr::Subscript {
                    value: ast::Expr::Name(var_decl_name.into()).into_boxed_node(),
                    index: ast::Expr::Num(index.to_string().into()).into_boxed_node(),
                }
                .into_node(),
                value: ast::Expr::Name(format!("val{}", index).into()).into_node(),
            }
            .into_node()
        })
        .collect::<Vec<_>>();

    // Build the AST node for the return statement
    let return_stmt = ast::FuncStmt::Return {
        value: Some(ast::Expr::Name(var_decl_name.into()).into_node()),
    }
    .into_node();

    let return_type = Some(build_type_desc(&Type::Array(array.clone())).into_node());

    // Put it all together in one AST node that holds the entire function definition
    ast::Function {
        pub_: None,
        unsafe_: None,
        name: names::list_expr_generator_fn_name(array).into_node(),
        args,
        generic_params: Vec::new().into_node(),
        return_type,
        body: [vec![var_decl], assignments, vec![return_stmt]].concat(),
    }
}
