mod test_db;
use std::path::Path;

use dir_test::{dir_test, Fixture};
use fe_compiler_test_utils::snap_test;
use fe_hir_analysis::{
    name_resolution::{DefConflictAnalysisPass, ImportAnalysisPass, PathAnalysisPass},
    ty::{
        ty_check::check_func_body, AdtDefAnalysisPass, BodyAnalysisPass, FuncAnalysisPass,
        ImplAnalysisPass, ImplTraitAnalysisPass, TraitAnalysisPass, TypeAliasAnalysisPass,
    },
};
use hir::{analysis_pass::AnalysisPassManager, lower::map_file_to_mod, LowerHirDb, ParsingPass};
use test_db::HirAnalysisTestDb;

#[dir_test(
    dir: "$CARGO_MANIFEST_DIR/test_files/ty_check",
    glob: "**/*.fe"
)]
fn test_standalone(fixture: Fixture<&str>) {
    let mut db = HirAnalysisTestDb::default();
    let path = Path::new(fixture.path());
    let file_name = path.file_name().and_then(|file| file.to_str()).unwrap();
    let input = db.new_stand_alone(file_name, fixture.content());
    let (top_mod, mut prop_formatter) = db.top_mod(input);

    db.assert_no_diags(top_mod);

    for &func in top_mod.all_funcs(&db) {
        let Some(body) = func.body(&db) else {
            continue;
        };

        let typed_body = &check_func_body(&db, func).1;
        for expr in body.exprs(&db).keys() {
            let ty = typed_body.expr_ty(&db, expr);
            prop_formatter.push_prop(
                func.top_mod(&db),
                expr.lazy_span(body).into(),
                ty.pretty_print(&db).to_string(),
            );
        }

        for pat in body.pats(&db).keys() {
            let ty = typed_body.pat_ty(&db, pat);
            prop_formatter.push_prop(
                func.top_mod(&db),
                pat.lazy_span(body).into(),
                ty.pretty_print(&db).to_string(),
            );
        }
    }

    let res = prop_formatter.finish(&db);
    snap_test!(res, fixture.path());
}

#[test]
fn test_updated() {
    let mut db = HirAnalysisTestDb::default();
    let file_name = "file.fe";
    let versions = vec![
        r#"fn foo() {}"#,
        r#"use bla
           fn foo() {}"#,
        r#"use bla::bla
           fn foo() {}"#,
        r#"use bla::bla::bla
           fn foo() {}"#,
        r#"use bla::bla::bla::bla
           fn foo() {}"#,
    ];

    let input = db.new_stand_alone(file_name, versions[0]);

    for _ in 0..10 {
        for version in &versions {
            {
                let top_mod = map_file_to_mod(db.as_lower_hir_db(), input);
                let mut pass_manager = initialize_pass_manager(&db);
                let _ = pass_manager.run_on_module(top_mod);
            }

            {
                input.set_text(&mut db).to(version.to_string());
            }
        }
    }
}

fn initialize_pass_manager(db: &HirAnalysisTestDb) -> AnalysisPassManager<'_> {
    let mut pass_manager = AnalysisPassManager::new();
    pass_manager.add_module_pass(Box::new(ParsingPass::new(db)));
    pass_manager.add_module_pass(Box::new(DefConflictAnalysisPass::new(db)));
    pass_manager.add_module_pass(Box::new(ImportAnalysisPass::new(db)));
    pass_manager.add_module_pass(Box::new(PathAnalysisPass::new(db)));
    pass_manager.add_module_pass(Box::new(AdtDefAnalysisPass::new(db)));
    pass_manager.add_module_pass(Box::new(TypeAliasAnalysisPass::new(db)));
    pass_manager.add_module_pass(Box::new(TraitAnalysisPass::new(db)));
    pass_manager.add_module_pass(Box::new(ImplAnalysisPass::new(db)));
    pass_manager.add_module_pass(Box::new(ImplTraitAnalysisPass::new(db)));
    pass_manager.add_module_pass(Box::new(FuncAnalysisPass::new(db)));
    pass_manager.add_module_pass(Box::new(BodyAnalysisPass::new(db)));
    pass_manager
}
