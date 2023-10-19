use std::collections::BTreeSet;

use common::diagnostics::{
    CompleteDiagnostic, DiagnosticPass, GlobalErrorCode, LabelStyle, Severity, SubDiagnostic,
};
use hir::{
    diagnostics::DiagnosticVoucher,
    hir_def::{IdentId, ImplTrait, Trait, TypeAlias as HirTypeAlias},
    span::{DynLazySpan, LazySpan},
    HirDb, SpannedHirDb,
};

use crate::HirAnalysisDb;

use super::{
    constraint::PredicateId,
    ty_def::{Kind, TyId},
};

use itertools::Itertools;

#[salsa::accumulator]
pub struct AdtDefDiagAccumulator(pub(super) TyDiagCollection);
#[salsa::accumulator]
pub struct TraitDefDiagAccumulator(pub(super) TyDiagCollection);
#[salsa::accumulator]
pub struct ImplTraitDefDiagAccumulator(pub(super) TyDiagCollection);
#[salsa::accumulator]
pub struct ImplDefDiagAccumulator(pub(super) TyDiagCollection);
#[salsa::accumulator]
pub struct TypeAliasDefDiagAccumulator(pub(super) TyDiagCollection);

#[derive(Debug, PartialEq, Eq, Hash, Clone, derive_more::From)]
pub enum TyDiagCollection {
    Ty(TyLowerDiag),
    Satisfaction(TraitConstraintDiag),
    TraitLower(TraitLowerDiag),
    Impl(ImplDiag),
}

impl TyDiagCollection {
    pub(super) fn to_voucher(&self) -> Box<dyn hir::diagnostics::DiagnosticVoucher> {
        match self.clone() {
            TyDiagCollection::Ty(diag) => Box::new(diag) as _,
            TyDiagCollection::Satisfaction(diag) => Box::new(diag) as _,
            TyDiagCollection::TraitLower(diag) => Box::new(diag) as _,
            TyDiagCollection::Impl(diag) => Box::new(diag) as _,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TyLowerDiag {
    NotFullyAppliedType(DynLazySpan),
    InvalidTypeArgKind(DynLazySpan, String),
    RecursiveType {
        primary_span: DynLazySpan,
        field_span: DynLazySpan,
    },

    UnboundTypeAliasParam {
        span: DynLazySpan,
        type_alias: HirTypeAlias,
        n_given_arg: usize,
    },
    TypeAliasCycle {
        primary: DynLazySpan,
        cycle: Vec<HirTypeAlias>,
    },

    InconsistentKindBound(DynLazySpan, String),

    KindBoundNotAllowed(DynLazySpan),

    AssocTy(DynLazySpan),
}

impl TyLowerDiag {
    pub fn not_fully_applied_type(span: DynLazySpan) -> Self {
        Self::NotFullyAppliedType(span)
    }

    pub fn invalid_type_arg_kind(
        db: &dyn HirAnalysisDb,
        span: DynLazySpan,
        expected: Option<Kind>,
        arg: TyId,
    ) -> Self {
        let msg = if let Some(expected) = expected {
            let arg_kind = arg.kind(db);
            debug_assert!(!expected.does_match(arg_kind));

            format!(
                "expected `{}` kind, but `{}` has `{}` kind",
                expected,
                arg.pretty_print(db),
                arg_kind
            )
        } else {
            "too many generic arguments".to_string()
        };

        Self::InvalidTypeArgKind(span, msg)
    }

    pub(super) fn recursive_type(primary_span: DynLazySpan, field_span: DynLazySpan) -> Self {
        Self::RecursiveType {
            primary_span,
            field_span,
        }
    }

    pub(super) fn unbound_type_alias_param(
        span: DynLazySpan,
        type_alias: HirTypeAlias,
        n_given_arg: usize,
    ) -> Self {
        Self::UnboundTypeAliasParam {
            span,
            type_alias,
            n_given_arg,
        }
    }

    pub(super) fn inconsistent_kind_bound(
        db: &dyn HirAnalysisDb,
        span: DynLazySpan,
        ty: TyId,
        former_bound: &Kind,
        new_kind: &Kind,
    ) -> Self {
        let msg = format!(
            "`{}` is already declared with `{}` kind, but found `{}` kind here",
            ty.pretty_print(db),
            former_bound,
            new_kind
        );
        Self::InconsistentKindBound(span, msg)
    }

    pub(super) fn assoc_ty(span: DynLazySpan) -> Self {
        Self::AssocTy(span)
    }

    fn local_code(&self) -> u16 {
        match self {
            Self::NotFullyAppliedType(_) => 0,
            Self::InvalidTypeArgKind(_, _) => 1,
            Self::RecursiveType { .. } => 2,
            Self::UnboundTypeAliasParam { .. } => 3,
            Self::TypeAliasCycle { .. } => 4,
            Self::InconsistentKindBound(_, _) => 5,
            Self::KindBoundNotAllowed(_) => 6,
            Self::AssocTy(_) => 7,
        }
    }

    fn message(&self) -> String {
        match self {
            Self::NotFullyAppliedType(_) => "expected fully applied type".to_string(),
            Self::InvalidTypeArgKind(_, _) => "invalid type argument kind".to_string(),
            Self::RecursiveType { .. } => "recursive type is not allowed".to_string(),

            Self::UnboundTypeAliasParam { .. } => {
                "all type parameters of type alias must be given".to_string()
            }
            Self::TypeAliasCycle { .. } => "recursive type alias cycle is detected".to_string(),

            Self::InconsistentKindBound(_, _) => "duplicate type bound is not allowed.".to_string(),
            Self::KindBoundNotAllowed(_) => "kind bound is not allowed".to_string(),

            Self::AssocTy(_) => "associated type is not supported ".to_string(),
        }
    }

    fn sub_diags(&self, db: &dyn SpannedHirDb) -> Vec<SubDiagnostic> {
        match self {
            Self::NotFullyAppliedType(span) => vec![SubDiagnostic::new(
                LabelStyle::Primary,
                "expected fully applied type here".to_string(),
                span.resolve(db),
            )],

            Self::InvalidTypeArgKind(span, msg) => vec![SubDiagnostic::new(
                LabelStyle::Primary,
                msg.clone(),
                span.resolve(db),
            )],

            Self::RecursiveType {
                primary_span,
                field_span,
            } => {
                vec![
                    SubDiagnostic::new(
                        LabelStyle::Primary,
                        "recursive type definition".to_string(),
                        primary_span.resolve(db),
                    ),
                    SubDiagnostic::new(
                        LabelStyle::Secondary,
                        "recursion occurs here".to_string(),
                        field_span.resolve(db),
                    ),
                ]
            }

            Self::UnboundTypeAliasParam {
                span: primary_span,
                type_alias,
                ..
            } => {
                vec![
                    SubDiagnostic::new(
                        LabelStyle::Primary,
                        format!(
                            "expected at least {} arguments here",
                            type_alias
                                .generic_params(db.as_hir_db())
                                .len(db.as_hir_db())
                        ),
                        primary_span.resolve(db),
                    ),
                    SubDiagnostic::new(
                        LabelStyle::Secondary,
                        "type alias defined here".to_string(),
                        type_alias.lazy_span().resolve(db),
                    ),
                ]
            }

            Self::TypeAliasCycle { primary, cycle } => {
                let mut diags = vec![SubDiagnostic::new(
                    LabelStyle::Primary,
                    "cycle happens here".to_string(),
                    primary.resolve(db),
                )];
                diags.extend(cycle.iter().map(|type_alias| {
                    SubDiagnostic::new(
                        LabelStyle::Secondary,
                        "type alias defined here".to_string(),
                        type_alias.lazy_span().alias_moved().resolve(db),
                    )
                }));
                diags
            }

            Self::InconsistentKindBound(primary, msg) => {
                vec![SubDiagnostic::new(
                    LabelStyle::Primary,
                    msg.clone(),
                    primary.resolve(db),
                )]
            }
            Self::KindBoundNotAllowed(span) => vec![SubDiagnostic::new(
                LabelStyle::Primary,
                "kind bound is not allowed here".to_string(),
                span.resolve(db),
            )],

            Self::AssocTy(span) => vec![SubDiagnostic::new(
                LabelStyle::Primary,
                "associated type is not implemented".to_string(),
                span.resolve(db),
            )],
        }
    }

    fn severity(&self) -> Severity {
        Severity::Error
    }
}

impl DiagnosticVoucher for TyLowerDiag {
    fn error_code(&self) -> GlobalErrorCode {
        GlobalErrorCode::new(DiagnosticPass::TypeDefinition, self.local_code())
    }

    fn to_complete(&self, db: &dyn SpannedHirDb) -> CompleteDiagnostic {
        let severity = self.severity();
        let error_code = self.error_code();
        let message = self.message();
        let sub_diags = self.sub_diags(db);

        CompleteDiagnostic::new(severity, message, sub_diags, vec![], error_code)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TraitLowerDiag {
    ExternalTraitForExternalType(DynLazySpan),

    ConflictTraitImpl {
        primary: ImplTrait,
        conflict_with: ImplTrait,
    },

    CyclicSuperTraits(DynLazySpan),
}

impl TraitLowerDiag {
    pub(super) fn external_trait_for_external_type(impl_trait: ImplTrait) -> Self {
        Self::ExternalTraitForExternalType(impl_trait.lazy_span().trait_ref().into())
    }

    pub(super) fn conflict_impl(primary: ImplTrait, conflict_with: ImplTrait) -> Self {
        Self::ConflictTraitImpl {
            primary,
            conflict_with,
        }
    }

    fn local_code(&self) -> u16 {
        match self {
            Self::ExternalTraitForExternalType(_) => 0,
            Self::ConflictTraitImpl { .. } => 1,
            Self::CyclicSuperTraits { .. } => 2,
        }
    }

    fn message(&self) -> String {
        match self {
            Self::ExternalTraitForExternalType(_) => {
                "external trait cannot be implemented for external type".to_string()
            }

            Self::ConflictTraitImpl { .. } => "conflict trait implementation".to_string(),

            Self::CyclicSuperTraits { .. } => "cyclic super traits are not allowed".to_string(),
        }
    }

    fn sub_diags(&self, db: &dyn hir::SpannedHirDb) -> Vec<SubDiagnostic> {
        match self {
            Self::ExternalTraitForExternalType(span) => vec![SubDiagnostic::new(
                LabelStyle::Primary,
                "external trait cannot be implemented for external type".to_string(),
                span.resolve(db),
            )],

            Self::ConflictTraitImpl {
                primary,
                conflict_with,
            } => vec![
                SubDiagnostic::new(
                    LabelStyle::Primary,
                    "conflict trait implementation".to_string(),
                    primary.lazy_span().ty().resolve(db),
                ),
                SubDiagnostic::new(
                    LabelStyle::Secondary,
                    "conflict with this trait implementation".to_string(),
                    conflict_with.lazy_span().ty().resolve(db),
                ),
            ],

            Self::CyclicSuperTraits(span) => {
                vec![SubDiagnostic::new(
                    LabelStyle::Primary,
                    "super traits cycle is detected here".to_string(),
                    span.resolve(db),
                )]
            }
        }
    }

    fn severity(&self) -> Severity {
        Severity::Error
    }
}

impl DiagnosticVoucher for TraitLowerDiag {
    fn error_code(&self) -> GlobalErrorCode {
        GlobalErrorCode::new(DiagnosticPass::ImplTraitDefinition, self.local_code())
    }

    fn to_complete(&self, db: &dyn hir::SpannedHirDb) -> CompleteDiagnostic {
        let severity = self.severity();
        let error_code = self.error_code();
        let message = self.message();
        let sub_diags = self.sub_diags(db);

        CompleteDiagnostic::new(severity, message, sub_diags, vec![], error_code)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TraitConstraintDiag {
    KindMismatch {
        primary: DynLazySpan,
        trait_def: Trait,
    },

    TraitArgNumMismatch {
        span: DynLazySpan,
        expected: usize,
        given: usize,
    },

    TraitArgKindMismatch(DynLazySpan, String),

    TraitBoundNotSat(DynLazySpan, String),

    InfiniteBoundRecursion(DynLazySpan, String),

    ConcreteTypeBound(DynLazySpan, String),
}

impl TraitConstraintDiag {
    pub(super) fn kind_mismatch(
        db: &dyn HirAnalysisDb,
        span: DynLazySpan,
        expected: &Kind,
        actual: TyId,
    ) -> Self {
        let actual_kind = actual.kind(db);
        let ty_display = actual.pretty_print(db);
        let msg = format!(
            "expected `{}` kind, but `{}` has `{}` kind",
            expected, ty_display, actual_kind,
        );
        Self::TraitArgKindMismatch(span, msg)
    }

    pub(super) fn trait_arg_num_mismatch(span: DynLazySpan, expected: usize, given: usize) -> Self {
        Self::TraitArgNumMismatch {
            span,
            expected,
            given,
        }
    }

    pub(super) fn trait_bound_not_satisfied(
        db: &dyn HirAnalysisDb,
        span: DynLazySpan,
        pred: PredicateId,
    ) -> Self {
        let ty = pred.ty(db);
        let goal = pred.trait_inst(db);
        let msg = format!(
            "`{}` doesn't implement `{}`",
            ty.pretty_print(db),
            goal.pretty_print(db)
        );
        Self::TraitBoundNotSat(span, msg)
    }

    pub(super) fn infinite_bound_recursion(
        db: &dyn HirAnalysisDb,
        span: DynLazySpan,
        pred: PredicateId,
    ) -> Self {
        let goal = pred.trait_inst(db);
        let ty = pred.ty(db);
        let msg = format!(
            "infinite evaluation recursion occurs when checking `{}: {}` ",
            ty.pretty_print(db),
            goal.pretty_print(db)
        );
        Self::InfiniteBoundRecursion(span, msg)
    }

    pub(super) fn concrete_type_bound(db: &dyn HirAnalysisDb, span: DynLazySpan, ty: TyId) -> Self {
        let msg = format!("`{}` is a concrete type", ty.pretty_print(db));
        Self::ConcreteTypeBound(span, msg)
    }

    fn local_code(&self) -> u16 {
        match self {
            Self::KindMismatch { .. } => 0,
            Self::TraitArgNumMismatch { .. } => 1,
            Self::TraitArgKindMismatch(_, _) => 2,
            Self::TraitBoundNotSat(_, _) => 3,
            Self::InfiniteBoundRecursion(_, _) => 4,
            Self::ConcreteTypeBound(_, _) => 5,
        }
    }

    fn message(&self) -> String {
        match self {
            Self::KindMismatch { .. } => "type doesn't satisfy required kind bound".to_string(),

            Self::TraitArgNumMismatch { .. } => "given trait argument number mismatch".to_string(),

            Self::TraitArgKindMismatch(_, _) => "given trait argument kind mismatch".to_string(),

            Self::TraitBoundNotSat(_, _) => "trait bound is not satisfied".to_string(),

            Self::InfiniteBoundRecursion(_, _) => "infinite trait bound recursion".to_string(),

            Self::ConcreteTypeBound(_, _) => {
                "trait bound for concrete type is not allowed".to_string()
            }
        }
    }

    fn sub_diags(&self, db: &dyn SpannedHirDb) -> Vec<SubDiagnostic> {
        match self {
            Self::KindMismatch { primary, trait_def } => vec![
                SubDiagnostic::new(
                    LabelStyle::Primary,
                    "type doesn't satisfy required kind bound here".to_string(),
                    primary.resolve(db),
                ),
                SubDiagnostic::new(
                    LabelStyle::Secondary,
                    "trait is defined here".to_string(),
                    trait_def.lazy_span().name().resolve(db),
                ),
            ],

            Self::TraitArgNumMismatch {
                span,
                expected,
                given,
            } => {
                vec![SubDiagnostic::new(
                    LabelStyle::Primary,
                    format!("expected {} arguments here, but {} given", expected, given,),
                    span.resolve(db),
                )]
            }

            Self::TraitArgKindMismatch(span, msg) => vec![SubDiagnostic::new(
                LabelStyle::Primary,
                msg.clone(),
                span.resolve(db),
            )],

            Self::TraitBoundNotSat(span, msg) => vec![SubDiagnostic::new(
                LabelStyle::Primary,
                msg.clone(),
                span.resolve(db),
            )],

            Self::InfiniteBoundRecursion(span, msg) => vec![SubDiagnostic::new(
                LabelStyle::Primary,
                msg.clone(),
                span.resolve(db),
            )],

            Self::ConcreteTypeBound(span, msg) => vec![SubDiagnostic::new(
                LabelStyle::Primary,
                msg.clone(),
                span.resolve(db),
            )],
        }
    }

    fn severity(&self) -> Severity {
        Severity::Error
    }
}

impl DiagnosticVoucher for TraitConstraintDiag {
    fn error_code(&self) -> GlobalErrorCode {
        GlobalErrorCode::new(DiagnosticPass::TraitSatisfaction, self.local_code())
    }

    fn to_complete(&self, db: &dyn SpannedHirDb) -> CompleteDiagnostic {
        let severity = self.severity();
        let error_code = self.error_code();
        let message = self.message();
        let sub_diags = self.sub_diags(db);

        CompleteDiagnostic::new(severity, message, sub_diags, vec![], error_code)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ImplDiag {
    ConflictMethodImpl {
        primary: DynLazySpan,
        conflict_with: DynLazySpan,
    },

    MethodNotDefinedInTrait {
        primary: DynLazySpan,
        trait_: Trait,
        method_name: IdentId,
    },

    NotAllTraitItemsImplemented {
        primary: DynLazySpan,
        not_implemented: BTreeSet<IdentId>,
    },

    MethodTypeParamNumMismatch {
        primary: DynLazySpan,
        expected: usize,
        given: usize,
    },

    MethodTypeParamKindMismatch {
        primary: DynLazySpan,
        message: String,
    },

    MethodArgNumMismatch {
        primary: DynLazySpan,
        expected: usize,
        given: usize,
    },

    MethodArgTyMismatch {
        primary: DynLazySpan,
        message: String,
    },

    MethodRetTyMismatch {
        primary: DynLazySpan,
        message: String,
    },

    MethodStricterBound {
        primary: DynLazySpan,
        message: String,
    },
}

impl ImplDiag {
    pub(super) fn conflict_method_impl(primary: DynLazySpan, conflict_with: DynLazySpan) -> Self {
        Self::ConflictMethodImpl {
            primary,
            conflict_with,
        }
    }

    pub(super) fn method_not_defined_in_trait(
        primary: DynLazySpan,
        trait_: Trait,
        method_name: IdentId,
    ) -> Self {
        Self::MethodNotDefinedInTrait {
            primary,
            trait_,
            method_name,
        }
    }

    pub(super) fn not_all_trait_items_implemented(
        primary: DynLazySpan,
        not_implemented: BTreeSet<IdentId>,
    ) -> Self {
        Self::NotAllTraitItemsImplemented {
            primary,
            not_implemented,
        }
    }

    pub(super) fn method_param_num_mismatch(
        primary: DynLazySpan,
        expected: usize,
        given: usize,
    ) -> Self {
        Self::MethodTypeParamNumMismatch {
            primary,
            expected,
            given,
        }
    }

    pub(super) fn method_arg_num_mismatch(
        primary: DynLazySpan,
        expected: usize,
        given: usize,
    ) -> Self {
        Self::MethodArgNumMismatch {
            primary,
            expected,
            given,
        }
    }

    pub fn method_arg_ty_mismatch(
        db: &dyn HirAnalysisDb,
        primary: DynLazySpan,
        expected: TyId,
        given: TyId,
    ) -> Self {
        let message = format!(
            "expected `{}` type, but the given type is `{}`",
            expected.pretty_print(db),
            given.pretty_print(db),
        );

        Self::MethodArgTyMismatch { primary, message }
    }

    pub fn method_ret_type_mismatch(
        db: &dyn HirAnalysisDb,
        primary: DynLazySpan,
        expected: TyId,
        given: TyId,
    ) -> Self {
        let message = format!(
            "expected `{}` type, but the given type is `{}`",
            expected.pretty_print(db),
            given.pretty_print(db),
        );

        Self::MethodRetTyMismatch { primary, message }
    }

    pub(super) fn method_param_kind_mismatch(
        primary: DynLazySpan,
        expected: &Kind,
        given: &Kind,
    ) -> Self {
        let message = format!(
            "expected `{}` kind, but the given type has `{}` kind",
            expected, given,
        );

        Self::MethodTypeParamKindMismatch { primary, message }
    }

    pub(super) fn method_stricter_bound(
        db: &dyn HirAnalysisDb,
        primary: DynLazySpan,
        stricter_bounds: &[PredicateId],
    ) -> Self {
        let message = format!(
            "method has stricter bounds than the declared method in the trait: {}",
            stricter_bounds
                .iter()
                .map(|pred| format!("`{}`", pred.pretty_print(db)))
                .join(", ")
        );
        Self::MethodStricterBound { primary, message }
    }

    pub fn local_code(&self) -> u16 {
        match self {
            Self::ConflictMethodImpl { .. } => 0,
            Self::MethodNotDefinedInTrait { .. } => 1,
            Self::NotAllTraitItemsImplemented { .. } => 2,
            Self::MethodTypeParamNumMismatch { .. } => 3,
            Self::MethodTypeParamKindMismatch { .. } => 4,
            Self::MethodArgNumMismatch { .. } => 5,
            Self::MethodArgTyMismatch { .. } => 6,
            Self::MethodRetTyMismatch { .. } => 7,
            Self::MethodStricterBound { .. } => 8,
        }
    }

    fn message(&self, db: &dyn HirDb) -> String {
        match self {
            Self::ConflictMethodImpl { .. } => "conflict method implementation".to_string(),
            Self::MethodNotDefinedInTrait {
                trait_,
                method_name,
                ..
            } => format!(
                "method `{}` is not defined in trait `{}`",
                method_name.data(db),
                trait_.name(db).unwrap().data(db),
            ),

            Self::NotAllTraitItemsImplemented { .. } => {
                "not all trait methods are implemented".to_string()
            }

            Self::MethodTypeParamNumMismatch { .. } => {
                "trait method type parameter number mismatch".to_string()
            }

            Self::MethodTypeParamKindMismatch { .. } => {
                "trait method type parameter kind mismatch".to_string()
            }

            Self::MethodArgNumMismatch { .. } => {
                "trait method argument number mismatch".to_string()
            }

            Self::MethodArgTyMismatch { .. } => {
                "given argument type doesn't match the expected type required by trait".to_string()
            }

            Self::MethodRetTyMismatch { .. } => {
                "given return type doesn't match the expected type required by trait".to_string()
            }

            Self::MethodStricterBound { .. } => {
                "impl method has stricter bound than the declared method in the trait".to_string()
            }
        }
    }

    fn sub_diags(&self, db: &dyn SpannedHirDb) -> Vec<SubDiagnostic> {
        match self {
            Self::ConflictMethodImpl {
                primary,
                conflict_with,
            } => vec![
                SubDiagnostic::new(
                    LabelStyle::Primary,
                    "conflict method implementation".to_string(),
                    primary.resolve(db),
                ),
                SubDiagnostic::new(
                    LabelStyle::Secondary,
                    "conflict with this method implementation".to_string(),
                    conflict_with.resolve(db),
                ),
            ],

            Self::MethodNotDefinedInTrait {
                primary,
                trait_,
                method_name,
            } => {
                vec![
                    SubDiagnostic::new(
                        LabelStyle::Primary,
                        format!(
                            "method `{}` is not defined in trait `{}`",
                            method_name.data(db.as_hir_db()),
                            trait_.name(db.as_hir_db()).unwrap().data(db.as_hir_db()),
                        ),
                        primary.resolve(db),
                    ),
                    SubDiagnostic::new(
                        LabelStyle::Secondary,
                        "trait is defined here".to_string(),
                        trait_.lazy_span().name().resolve(db),
                    ),
                ]
            }

            Self::NotAllTraitItemsImplemented {
                primary,
                not_implemented,
            } => {
                let not_implemented: String = not_implemented
                    .iter()
                    .map(|name| name.data(db.as_hir_db()))
                    .join(", ");

                vec![SubDiagnostic::new(
                    LabelStyle::Primary,
                    format!(
                        "all required trait items must be implemented, missing: `{}`",
                        not_implemented
                    ),
                    primary.resolve(db),
                )]
            }

            Self::MethodTypeParamNumMismatch {
                primary,
                expected,
                given,
            } => {
                vec![SubDiagnostic::new(
                    LabelStyle::Primary,
                    format!(
                        "expected {} type parameters here, but {} given",
                        expected, given
                    ),
                    primary.resolve(db),
                )]
            }

            Self::MethodTypeParamKindMismatch { primary, message } => {
                vec![SubDiagnostic::new(
                    LabelStyle::Primary,
                    message.clone(),
                    primary.resolve(db),
                )]
            }

            Self::MethodArgNumMismatch {
                primary,
                expected,
                given,
            } => {
                vec![SubDiagnostic::new(
                    LabelStyle::Primary,
                    format!("expected {} arguments here, but {} given", expected, given),
                    primary.resolve(db),
                )]
            }

            Self::MethodArgTyMismatch { primary, message } => {
                vec![SubDiagnostic::new(
                    LabelStyle::Primary,
                    message.clone(),
                    primary.resolve(db),
                )]
            }

            Self::MethodRetTyMismatch { primary, message } => {
                vec![SubDiagnostic::new(
                    LabelStyle::Primary,
                    message.clone(),
                    primary.resolve(db),
                )]
            }

            Self::MethodStricterBound { primary, message } => {
                vec![SubDiagnostic::new(
                    LabelStyle::Primary,
                    message.clone(),
                    primary.resolve(db),
                )]
            }
        }
    }

    fn severity(&self) -> Severity {
        Severity::Error
    }
}

impl DiagnosticVoucher for ImplDiag {
    fn error_code(&self) -> GlobalErrorCode {
        GlobalErrorCode::new(DiagnosticPass::TraitSatisfaction, self.local_code())
    }

    fn to_complete(&self, db: &dyn SpannedHirDb) -> CompleteDiagnostic {
        let severity = self.severity();
        let error_code = self.error_code();
        let message = self.message(db.as_hir_db());
        let sub_diags = self.sub_diags(db);

        CompleteDiagnostic::new(severity, message, sub_diags, vec![], error_code)
    }
}
