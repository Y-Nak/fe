use crate::{
    hir_def::{
        scope_graph::{
            EdgeKind, LocalScope, LocalScopeId, ScopeEdge, ScopeGraph, ScopeId, ScopeKind,
        },
        EnumVariantListId, FnParamListId, FnParamName, GenericParamListId, ItemKind,
        RecordFieldListId, TopLevelMod, Visibility,
    },
    HirDb,
};

pub struct ScopeGraphBuilder<'db> {
    pub(super) db: &'db dyn HirDb,
    pub(super) top_mod: TopLevelMod,
    graph: ScopeGraph,
    scope_stack: Vec<LocalScopeId>,
    module_stack: Vec<LocalScopeId>,
}

impl<'db> ScopeGraphBuilder<'db> {
    pub(crate) fn enter_top_mod(db: &'db dyn HirDb, top_mod: TopLevelMod) -> Self {
        let mut builder = Self {
            db,
            top_mod,
            graph: ScopeGraph {
                top_mod,
                scopes: Default::default(),
                item_map: Default::default(),
                unresolved_uses: Default::default(),
            },
            scope_stack: Default::default(),
            module_stack: Default::default(),
        };

        builder.enter_scope(true);
        builder
    }

    pub fn build(self) -> ScopeGraph {
        debug_assert!(matches!(
            self.graph.scope_item(LocalScopeId::root()),
            Some(ItemKind::TopMod(_))
        ));

        self.graph
    }

    pub fn enter_scope(&mut self, is_mod: bool) {
        // Create dummy scope, the scope kind is initialized in `leave_scope`.
        let id = self.graph.scopes.push(self.dummy_scope());
        self.scope_stack.push(id);
        if is_mod {
            self.module_stack.push(id);
        }
    }

    pub fn leave_scope(&mut self, item: ItemKind) {
        use ItemKind::*;

        let item_scope = self.scope_stack.pop().unwrap();
        self.initialize_item_scope(item_scope, item);

        if let ItemKind::TopMod(top_mod) = item {
            debug_assert!(self.scope_stack.is_empty());
            self.add_local_edge(item_scope, item_scope, EdgeKind::self_());

            self.add_global_edge(
                item_scope,
                top_mod.ingot(self.db).root_mod(self.db),
                EdgeKind::ingot(),
            );
            for child in top_mod.children(self.db) {
                let child_name = child.name(self.db);
                let edge = EdgeKind::mod_(child_name);
                self.add_global_edge(item_scope, child, edge)
            }

            if let Some(parent) = top_mod.parent(self.db) {
                let edge = EdgeKind::super_();
                self.add_global_edge(item_scope, parent, edge);
            }
            self.module_stack.pop().unwrap();

            return;
        }

        let parent_scope = *self.scope_stack.last().unwrap();
        let parent_to_child_edge = match item {
            Mod(inner) => {
                self.module_stack.pop().unwrap();

                self.add_local_edge(
                    item_scope,
                    *self.module_stack.last().unwrap(),
                    EdgeKind::super_(),
                );
                self.add_global_edge(
                    item_scope,
                    self.top_mod.ingot(self.db).root_mod(self.db),
                    EdgeKind::ingot(),
                );
                self.add_local_edge(item_scope, item_scope, EdgeKind::self_());

                inner
                    .name(self.db)
                    .to_opt()
                    .map(EdgeKind::mod_)
                    .unwrap_or_else(EdgeKind::anon)
            }

            Func(inner) => {
                self.add_lex_edge(item_scope, parent_scope);
                self.add_generic_param_scope(item_scope, inner.generic_params(self.db));
                if let Some(params) = inner.params(self.db).to_opt() {
                    self.add_func_param_scope(item_scope, params);
                }
                inner
                    .name(self.db)
                    .to_opt()
                    .map(EdgeKind::value)
                    .unwrap_or_else(EdgeKind::anon)
            }

            Struct(inner) => {
                self.add_lex_edge(item_scope, parent_scope);
                self.add_field_scope(item_scope, inner.fields(self.db));
                self.add_generic_param_scope(item_scope, inner.generic_params(self.db));
                inner
                    .name(self.db)
                    .to_opt()
                    .map(EdgeKind::type_)
                    .unwrap_or_else(EdgeKind::anon)
            }

            Contract(inner) => {
                self.add_lex_edge(item_scope, parent_scope);
                self.add_field_scope(item_scope, inner.fields(self.db));
                inner
                    .name(self.db)
                    .to_opt()
                    .map(EdgeKind::type_)
                    .unwrap_or_else(EdgeKind::anon)
            }

            Enum(inner) => {
                self.add_lex_edge(item_scope, parent_scope);
                let vis = inner.vis(self.db);
                self.add_variant_scope(item_scope, vis, inner.variants(self.db));
                self.add_generic_param_scope(item_scope, inner.generic_params(self.db));
                inner
                    .name(self.db)
                    .to_opt()
                    .map(EdgeKind::type_)
                    .unwrap_or_else(EdgeKind::anon)
            }

            TypeAlias(inner) => {
                self.add_lex_edge(item_scope, parent_scope);
                self.add_generic_param_scope(item_scope, inner.generic_params(self.db));
                inner
                    .name(self.db)
                    .to_opt()
                    .map(EdgeKind::type_)
                    .unwrap_or_else(EdgeKind::anon)
            }

            Impl(inner) => {
                self.add_lex_edge(item_scope, parent_scope);
                self.add_generic_param_scope(item_scope, inner.generic_params(self.db));
                self.add_local_edge(item_scope, item_scope, EdgeKind::self_ty());
                EdgeKind::anon()
            }

            Trait(inner) => {
                self.add_lex_edge(item_scope, parent_scope);
                self.add_generic_param_scope(item_scope, inner.generic_params(self.db));
                self.add_local_edge(item_scope, item_scope, EdgeKind::self_ty());
                inner
                    .name(self.db)
                    .to_opt()
                    .map(EdgeKind::trait_)
                    .unwrap_or_else(EdgeKind::anon)
            }

            ImplTrait(inner) => {
                self.add_lex_edge(item_scope, parent_scope);
                self.add_generic_param_scope(item_scope, inner.generic_params(self.db));
                self.add_local_edge(item_scope, item_scope, EdgeKind::self_ty());
                EdgeKind::anon()
            }

            Const(inner) => {
                self.add_lex_edge(item_scope, parent_scope);
                inner
                    .name(self.db)
                    .to_opt()
                    .map(EdgeKind::value)
                    .unwrap_or_else(EdgeKind::anon)
            }

            Use(use_) => {
                self.graph.unresolved_uses.push(use_);

                self.add_lex_edge(item_scope, parent_scope);
                EdgeKind::anon()
            }

            Body(_) => {
                self.add_lex_edge(item_scope, parent_scope);
                EdgeKind::anon()
            }

            _ => unreachable!(),
        };

        self.add_local_edge(parent_scope, item_scope, parent_to_child_edge);
    }

    fn initialize_item_scope(&mut self, scope: LocalScopeId, item: ItemKind) {
        self.graph.scopes[scope].kind = ScopeKind::Item(item);
        self.graph.scopes[scope].vis = item.vis(self.db);
        self.graph.item_map.insert(item, scope);
    }

    fn add_field_scope(&mut self, parent_scope: LocalScopeId, fields: RecordFieldListId) {
        for (i, field) in fields.data(self.db).iter().enumerate() {
            let scope = LocalScope::new(
                ScopeKind::Field(i),
                self.parent_module_id(),
                Some(parent_scope),
                field.vis,
            );
            let field_scope = self.graph.scopes.push(scope);
            self.add_lex_edge(field_scope, parent_scope);
            let kind = field
                .name
                .to_opt()
                .map(EdgeKind::field)
                .unwrap_or_else(EdgeKind::anon);
            self.add_local_edge(parent_scope, field_scope, kind)
        }
    }

    fn add_variant_scope(
        &mut self,
        parent_scope: LocalScopeId,
        parent_vis: Visibility,
        variants: EnumVariantListId,
    ) {
        for (i, field) in variants.data(self.db).iter().enumerate() {
            let scope = LocalScope::new(
                ScopeKind::Variant(i),
                self.parent_module_id(),
                Some(parent_scope),
                parent_vis,
            );
            let variant_scope = self.graph.scopes.push(scope);
            self.add_lex_edge(variant_scope, parent_scope);
            let kind = field
                .name
                .to_opt()
                .map(EdgeKind::variant)
                .unwrap_or_else(EdgeKind::anon);
            self.add_local_edge(parent_scope, variant_scope, kind)
        }
    }

    fn add_func_param_scope(&mut self, parent_scope: LocalScopeId, params: FnParamListId) {
        for (i, param) in params.data(self.db).iter().enumerate() {
            let scope = LocalScope::new(
                ScopeKind::FnParam(i),
                self.parent_module_id(),
                Some(parent_scope),
                Visibility::Private,
            );
            let generic_param_scope = self.graph.scopes.push(scope);
            self.add_lex_edge(generic_param_scope, parent_scope);
            let kind = param
                .name
                .to_opt()
                .map(|name| match name {
                    FnParamName::Ident(ident) => EdgeKind::value(ident),
                    FnParamName::Underscore => EdgeKind::anon(),
                })
                .unwrap_or_else(EdgeKind::anon);
            self.add_local_edge(parent_scope, generic_param_scope, kind)
        }
    }

    fn add_generic_param_scope(&mut self, parent_scope: LocalScopeId, params: GenericParamListId) {
        for (i, param) in params.data(self.db).iter().enumerate() {
            let scope = LocalScope::new(
                ScopeKind::GenericParam(i),
                self.parent_module_id(),
                Some(parent_scope),
                Visibility::Private,
            );
            let generic_param_scope = self.graph.scopes.push(scope);
            self.add_lex_edge(generic_param_scope, parent_scope);
            let kind = param
                .name()
                .to_opt()
                .map(EdgeKind::generic_param)
                .unwrap_or_else(EdgeKind::anon);
            self.add_local_edge(parent_scope, generic_param_scope, kind)
        }
    }

    fn dummy_scope(&self) -> LocalScope {
        LocalScope::new(
            ScopeKind::Item(self.top_mod.into()),
            self.parent_module_id(),
            None,
            Visibility::Public,
        )
    }

    fn parent_module_id(&self) -> Option<ScopeId> {
        if let Some(id) = self.module_stack.last() {
            Some(ScopeId::new(self.top_mod, *id))
        } else {
            self.top_mod
                .parent(self.db)
                .map(|top_mod| ScopeId::new(top_mod, LocalScopeId::root()))
        }
    }

    fn add_lex_edge(&mut self, child: LocalScopeId, parent: LocalScopeId) {
        self.add_local_edge(child, parent, EdgeKind::lex());
        self.graph.scopes[child].parent_scope = Some(parent);
    }

    fn add_local_edge(&mut self, source: LocalScopeId, dest: LocalScopeId, kind: EdgeKind) {
        self.graph.scopes[source].edges.push(ScopeEdge {
            dest: ScopeId::new(self.top_mod, dest),
            kind,
        });
    }

    fn add_global_edge(&mut self, source: LocalScopeId, dest: TopLevelMod, kind: EdgeKind) {
        self.graph.scopes[source].edges.push(ScopeEdge {
            dest: ScopeId::new(dest, LocalScopeId::root()),
            kind,
        });
    }
}
