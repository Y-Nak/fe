use rustc_hash::FxHashMap;

use super::{
    const_ty::{ConstTyData, ConstTyId},
    fold::{TyFoldable, TyFolder},
    ty_def::{TyData, TyId, TyVar},
    unify::{InferenceKey, UnificationStore, UnificationTableBase},
};
use crate::{ty::ty_def::collect_variables, HirAnalysisDb};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Canonical<T> {
    pub value: T,
}

impl<T> Canonical<T>
where
    T: for<'db> TyFoldable<'db>,
{
    pub fn new(db: &dyn HirAnalysisDb, value: T) -> Self {
        let mut c = Canonicalizer::new(db);
        let value = value.fold_with(&mut c);
        Canonical { value }
    }

    /// Extracts the identity from the canonical value.
    ///
    /// This method initializes the unification table with new variables
    /// based on the canonical value and then returns the canonical value
    /// itself.
    ///
    /// # Parameters
    /// - `table`: The unification table to be initialized with new variables.
    ///
    /// # Returns
    /// The canonical value after initializing the unification table.
    ///
    /// # Panics
    /// This function will panic if the `table` is not empty.
    pub(super) fn extract_identity<S>(self, table: &mut UnificationTableBase<S>) -> T
    where
        S: UnificationStore,
    {
        assert!(table.is_empty());

        for var in collect_variables(table.db(), &self.value).iter() {
            table.new_var(var.sort, &var.kind);
        }

        self.value
    }

    /// Canonicalize a new solution that corresponds to the canonical query.
    /// This function creates a new solution for a canonical query by folding
    /// the provided solution with the unification table. It then constructs
    /// a substitution map from probed type variables to canonical type
    /// variables, and uses this map to canonicalize the solution.
    ///
    /// # Parameters
    /// - `table`: The unification table must be from the same environment as
    ///   the solution.
    /// - `solution`: The solution to be canonicalized.
    ///
    /// # Returns
    /// A `Solution<U>` where `U` is the type of the provided solution,
    /// canonicalized to the context of the canonical query.
    pub(super) fn canonicalize_solution<S, U>(
        &self,
        table: &mut UnificationTableBase<S>,
        solution: U,
    ) -> Solution<U>
    where
        T: Copy,
        S: UnificationStore,
        U: for<'db> TyFoldable<'db> + Clone,
    {
        let db = table.db();

        let solution = solution.fold_with(table);
        // Make the substitution so that it maps back from probed type variable to
        // canonical type variables.
        // `Probed type variable -> Canonical type variable`.
        let canonical_vars = collect_variables(db, &self.value)
            .into_iter()
            .filter_map(|var| {
                let ty = TyId::ty_var(db, var.sort, var.kind, var.key);
                let probed = ty.fold_with(table);
                if probed.is_ty_var(db) {
                    Some((probed, ty))
                } else {
                    None
                }
            });
        let mut canonicalizer = Canonicalizer {
            db,
            subst: canonical_vars.collect(),
        };

        Solution {
            value: solution.fold_with(&mut canonicalizer),
        }
    }
}

/// This type contains [`Canonical`] type and auxiliary information to map back
/// [`Solution`] that corresponds to [`Canonical`] query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Canonicalized<T> {
    pub value: Canonical<T>,
    // A substitution from canonical type variables to original type variables.
    subst: FxHashMap<TyId, TyId>,
}

impl<T> Canonicalized<T>
where
    T: for<'db> TyFoldable<'db>,
{
    pub fn new(db: &dyn HirAnalysisDb, value: T) -> Self {
        let mut canonicalizer = Canonicalizer::new(db);
        let value = value.fold_with(&mut canonicalizer);
        let map = canonicalizer
            .subst
            .into_iter()
            .map(|(orig_var, canonical_var)| (canonical_var, orig_var))
            .collect();
        Canonicalized {
            value: Canonical { value },
            subst: map,
        }
    }

    /// Extracts the solution from the canonicalized query.
    ///
    /// This method takes a unification table and a solution, and returns the
    /// solution in the context of the original query environment.
    ///
    /// # Parameters
    /// - `table`: The unification table in the original query environement.
    /// - `solution`: The solution to extract.
    ///
    /// # Returns
    /// The extracted solution in the context of the original query environment.
    pub fn extract_solution<U, S>(
        &self,
        table: &mut UnificationTableBase<S>,
        solution: Solution<U>,
    ) -> U
    where
        U: for<'db> TyFoldable<'db>,
        S: UnificationStore,
    {
        let map = self.subst.clone();
        let mut extractor = SolutionExtractor::new(table, map);
        solution.value.fold_with(&mut extractor)
    }
}

/// Represents a solution to a [`Canonical`] query.
///
/// This type guarantees:
/// 1. Any type variable in the solution that is unifiable with a type variable
///    from the [`Canonical`] query will be canonicalized to that variable.
/// 2. All other type variables are canonicalized in a consistent manner with
///    the [`Canonical`] type.
///
/// To extract the internal value into the environment where the query was
/// created, use [`Canonicalized::extract_solution`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Solution<T> {
    pub(super) value: T,
}

/// A struct that helps in converting types to their canonical form.
/// It maintains a mapping from original type variables to canonical variables.
struct Canonicalizer<'db> {
    db: &'db dyn HirAnalysisDb,
    // A substitution from original type variables to canonical variables.
    subst: FxHashMap<TyId, TyId>,
}

impl<'db> Canonicalizer<'db> {
    fn new(db: &'db dyn HirAnalysisDb) -> Self {
        Canonicalizer {
            db,
            subst: FxHashMap::default(),
        }
    }

    fn canonical_var(&mut self, var: &TyVar) -> TyVar {
        let key = self.subst.len() as u32;
        TyVar {
            sort: var.sort,
            kind: var.kind.clone(),
            key: InferenceKey(key),
        }
    }
}

impl<'db> TyFolder<'db> for Canonicalizer<'db> {
    fn db(&self) -> &'db dyn HirAnalysisDb {
        self.db
    }

    fn fold_ty(&mut self, ty: TyId) -> TyId {
        if let Some(&canonical) = self.subst.get(&ty) {
            return canonical;
        }

        match ty.data(self.db) {
            TyData::TyVar(var) => {
                let canonical_var = self.canonical_var(var);
                let canonical_ty = TyId::new(self.db, TyData::TyVar(canonical_var));

                self.subst.insert(ty, canonical_ty);
                canonical_ty
            }

            TyData::ConstTy(const_ty) => {
                if let ConstTyData::TyVar(var, const_ty_ty) = const_ty.data(self.db) {
                    let canonical_var = self.canonical_var(var);
                    let const_ty =
                        ConstTyId::new(self.db, ConstTyData::TyVar(canonical_var, *const_ty_ty));
                    let canonical_ty = TyId::const_ty(self.db, const_ty);

                    self.subst.insert(ty, canonical_ty);
                    canonical_ty
                } else {
                    ty.super_fold_with(self)
                }
            }

            _ => ty.super_fold_with(self),
        }
    }
}

struct SolutionExtractor<'a, 'db, S>
where
    S: UnificationStore,
{
    table: &'a mut UnificationTableBase<'db, S>,
    /// A subst from canonical type variables to the variables in the current
    /// env.
    subst: FxHashMap<TyId, TyId>,
}

impl<'a, 'db, S> SolutionExtractor<'a, 'db, S>
where
    S: UnificationStore,
{
    fn new(table: &'a mut UnificationTableBase<'db, S>, subst: FxHashMap<TyId, TyId>) -> Self {
        SolutionExtractor { table, subst }
    }
}

impl<'a, 'db, S> TyFolder<'db> for SolutionExtractor<'a, 'db, S>
where
    S: UnificationStore,
{
    fn db(&self) -> &'db dyn HirAnalysisDb {
        self.table.db()
    }

    fn fold_ty(&mut self, ty: TyId) -> TyId {
        if let Some(&ty) = self.subst.get(&ty) {
            return ty;
        }

        match ty.data(self.db()) {
            TyData::TyVar(var) => {
                let new_ty = self.table.new_var(var.sort, &var.kind);
                self.subst.insert(ty, new_ty);
                new_ty
            }

            TyData::ConstTy(const_ty) => {
                if let ConstTyData::TyVar(var, const_ty_ty) = const_ty.data(self.db()) {
                    let new_key = self.table.new_key(&var.kind, var.sort);
                    let new_ty = TyId::const_ty_var(self.db(), *const_ty_ty, new_key);
                    self.subst.insert(ty, new_ty);
                    new_ty
                } else {
                    ty.super_fold_with(self)
                }
            }

            _ => ty.super_fold_with(self),
        }
    }
}
