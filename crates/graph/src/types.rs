use serde::{Deserialize, Serialize};

/// A value that can cross a wire.
///
/// The name is the contract. If pandas returns a different kind, the node
/// errors and names both. Nothing is wrapped into a DataFrame to make a wire fit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TypeName {
    Dataframe,
    Series,
    Index,
    Scalar,
    List,
    Dict,
    Ndarray,
    Code,
}

impl TypeName {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dataframe => "dataframe",
            Self::Series => "series",
            Self::Index => "index",
            Self::Scalar => "scalar",
            Self::List => "list",
            Self::Dict => "dict",
            Self::Ndarray => "ndarray",
            Self::Code => "code",
        }
    }

    /// Short name drawn on a port. The wire still checks the real type.
    pub fn short_label(self) -> &'static str {
        match self {
            Self::Dataframe => "table",
            Self::Series => "column",
            Self::Index => "index",
            Self::Scalar => "value",
            Self::List => "list",
            Self::Dict => "map",
            Self::Ndarray => "array",
            Self::Code => "code",
        }
    }
}

impl std::fmt::Display for TypeName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
