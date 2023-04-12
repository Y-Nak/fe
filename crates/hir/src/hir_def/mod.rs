pub mod attr;
pub mod body;
pub mod expr;
pub mod item;
pub mod params;
pub mod pat;
pub mod path;
pub mod stmt;
pub mod types;
pub mod use_tree;

pub(crate) mod item_tree;
pub(crate) mod module_tree;

pub use attr::*;
pub use body::*;
pub use expr::*;
pub use item::*;
use num_bigint::BigUint;
pub use params::*;
pub use pat::*;
pub use path::*;
pub use stmt::*;
pub use types::*;
pub use use_tree::*;

pub use item_tree::*;
pub use module_tree::*;

use crate::HirDb;

#[salsa::interned]
pub struct IdentId {
    data: String,
}
impl IdentId {
    pub fn is_self(&self, db: &dyn HirDb) -> bool {
        // TODO: Keyword should be prefilled in the database.
        // ref: https://github.com/salsa-rs/salsa/pull/440
        self.data(db) == "self"
    }
}

#[salsa::interned]
pub struct IntegerId {
    #[return_ref]
    pub data: BigUint,
}

#[salsa::interned]
pub struct StringId {
    /// The text of the string literal, without the quotes.
    #[return_ref]
    pub data: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LitKind {
    Int(IntegerId),
    String(StringId),
    Bool(bool),
}

/// `Partial<T>` is a type that explicitly indicates the possibility that an HIR
/// node cannot be generated due to syntax errors in the source file.
///
/// If a node is `Partial::Absent`, it means that the corresponding AST either
/// does not exist or is erroneous. When a `Partial::Absent` is generated, the
/// relevant error is always generated by the parser, so in Analysis phases, it
/// can often be ignored.
///
/// This type is clearly distinguished from `Option<T>`. The
/// `Option<T>` type is used to hold syntactically valid optional nodes, while
/// `Partial<T>` means that a syntactically required element may be missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Partial<T> {
    Present(T),
    Absent,
}

impl<T> Partial<T> {
    pub fn unwrap(&self) -> &T {
        match self {
            Self::Present(value) => value,
            Self::Absent => panic!("unwrap called on absent value"),
        }
    }
}

impl<T> Default for Partial<T> {
    fn default() -> Self {
        Self::Absent
    }
}

impl<T> From<Option<T>> for Partial<T> {
    fn from(value: Option<T>) -> Self {
        if let Some(value) = value {
            Self::Present(value)
        } else {
            Self::Absent
        }
    }
}
