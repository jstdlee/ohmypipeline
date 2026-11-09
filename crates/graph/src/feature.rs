use crate::types::TypeName;

pub const DF: &[TypeName] = &[TypeName::Dataframe];
pub const SERIES: &[TypeName] = &[TypeName::Series];
pub const DF_OR_SERIES: &[TypeName] = &[TypeName::Dataframe, TypeName::Series];
pub const DICT_OR_LIST: &[TypeName] = &[TypeName::Dict, TypeName::List];
pub const SCALAR: &[TypeName] = &[TypeName::Scalar];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamKind {
    Int,
    Float,
    Bool,
    String,
    Enum,
    Column,
    Code,
}

#[derive(Clone, Copy, Debug)]
pub struct ParamDef {
    pub name: &'static str,
    pub kind: ParamKind,
    pub wireable: bool,
    pub wire_types: &'static [TypeName],
    pub options: &'static [&'static str],
    /// JSON text of the default literal.
    pub default_json: &'static str,
}

#[derive(Clone, Copy, Debug)]
pub struct PortDef {
    pub name: &'static str,
    pub accepts: &'static [TypeName],
    pub multi: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct FeatureDef {
    pub id: &'static str,
    pub title: &'static str,
    pub group: &'static str,
    pub inputs: &'static [PortDef],
    /// Concrete output when the feature always produces one type.
    pub output: Option<TypeName>,
    /// Types the output might be when the concrete type is not known yet.
    pub possible_outputs: &'static [TypeName],
    pub output_name: &'static str,
    pub params: &'static [ParamDef],
    /// Appends a dataframe input on this node only.
    pub can_add_input: bool,
    /// The node stores its own input list (a code block).
    pub inputs_are_user: bool,
}

const READ_CSV_PARAMS: &[ParamDef] = &[ParamDef {
    name: "path",
    kind: ParamKind::String,
    wireable: true,
    wire_types: &[TypeName::Scalar],
    options: &[],
    default_json: "\"\"",
}];

const HEAD_PARAMS: &[ParamDef] = &[ParamDef {
    name: "n",
    kind: ParamKind::Int,
    wireable: true,
    wire_types: SCALAR,
    options: &[],
    default_json: "5",
}];

const CHANGE_HEADERS_PARAMS: &[ParamDef] = &[ParamDef {
    name: "mapping",
    kind: ParamKind::String,
    wireable: true,
    wire_types: DICT_OR_LIST,
    options: &[],
    default_json: "[]",
}];

const QUERY_PARAMS: &[ParamDef] = &[ParamDef {
    name: "expr",
    kind: ParamKind::String,
    wireable: false,
    wire_types: &[],
    options: &[],
    default_json: "\"\"",
}];

const COLUMN_PARAMS: &[ParamDef] = &[ParamDef {
    name: "name",
    kind: ParamKind::Column,
    wireable: false,
    wire_types: &[],
    options: &[],
    default_json: "\"\"",
}];

const LITERAL_PARAMS: &[ParamDef] = &[
    ParamDef {
        name: "kind",
        kind: ParamKind::Enum,
        wireable: false,
        wire_types: &[],
        options: &["scalar", "list", "dict", "code"],
        default_json: "\"scalar\"",
    },
    ParamDef {
        name: "value",
        kind: ParamKind::String,
        wireable: false,
        wire_types: &[],
        options: &[],
        default_json: "\"\"",
    },
];

const PYTHON_PARAMS: &[ParamDef] = &[ParamDef {
    name: "code",
    kind: ParamKind::Code,
    wireable: true,
    wire_types: &[TypeName::Code],
    options: &[],
    default_json: "\"def run(df):\\n    return df\\n\"",
}];

const CHART_PARAMS: &[ParamDef] = &[
    ParamDef {
        name: "mark",
        kind: ParamKind::Enum,
        wireable: false,
        wire_types: &[],
        options: &["line", "bar", "area", "scatter", "histogram", "pie"],
        default_json: "\"line\"",
    },
    ParamDef {
        name: "x",
        kind: ParamKind::Column,
        wireable: false,
        wire_types: &[],
        options: &[],
        default_json: "\"\"",
    },
    ParamDef {
        name: "y",
        kind: ParamKind::Column,
        wireable: false,
        wire_types: &[],
        options: &[],
        default_json: "\"\"",
    },
    ParamDef {
        name: "series",
        kind: ParamKind::Column,
        wireable: false,
        wire_types: &[],
        options: &[],
        default_json: "\"\"",
    },
    ParamDef {
        name: "bins",
        kind: ParamKind::Int,
        wireable: false,
        wire_types: &[],
        options: &[],
        default_json: "24",
    },
];

const SORT_PARAMS: &[ParamDef] = &[
    ParamDef {
        name: "by",
        kind: ParamKind::Column,
        wireable: false,
        wire_types: &[],
        options: &[],
        default_json: "\"\"",
    },
    ParamDef {
        name: "ascending",
        kind: ParamKind::Bool,
        wireable: false,
        wire_types: &[],
        options: &[],
        default_json: "true",
    },
];

const MERGE_PARAMS: &[ParamDef] = &[
    ParamDef {
        name: "on",
        kind: ParamKind::Column,
        wireable: false,
        wire_types: &[],
        options: &[],
        default_json: "\"\"",
    },
    ParamDef {
        name: "how",
        kind: ParamKind::Enum,
        wireable: false,
        wire_types: &[],
        options: &["inner", "left", "right", "outer"],
        default_json: "\"inner\"",
    },
];

const IN_DF: PortDef = PortDef {
    name: "in",
    accepts: DF,
    multi: false,
};
const IN_DF_OR_SERIES: PortDef = PortDef {
    name: "in",
    accepts: DF_OR_SERIES,
    multi: false,
};
const IN_MULTI: PortDef = PortDef {
    name: "in",
    accepts: DF_OR_SERIES,
    multi: true,
};

/// The milestone catalog, plus the fan-in and preview actions the wire and
/// table rules name directly. A pandas call that is not here is absent.
pub static CATALOG: &[FeatureDef] = &[
    FeatureDef {
        id: "read_csv",
        title: "Read CSV",
        group: "IO",
        inputs: &[],
        output: Some(TypeName::Dataframe),
        possible_outputs: DF,
        output_name: "out",
        params: READ_CSV_PARAMS,
        can_add_input: false,
        inputs_are_user: false,
    },
    FeatureDef {
        id: "read_excel",
        title: "Read Excel",
        group: "IO",
        inputs: &[],
        output: Some(TypeName::Dataframe),
        possible_outputs: DF,
        output_name: "out",
        params: &[
            ParamDef {
                name: "path",
                kind: ParamKind::String,
                wireable: true,
                wire_types: SCALAR,
                options: &[],
                default_json: "\"\"",
            },
            ParamDef {
                name: "sheet",
                kind: ParamKind::String,
                wireable: false,
                wire_types: &[],
                options: &[],
                default_json: "\"\"",
            },
        ],
        can_add_input: false,
        inputs_are_user: false,
    },
    FeatureDef {
        id: "read_sql",
        title: "Read SQLite",
        group: "IO",
        inputs: &[],
        output: Some(TypeName::Dataframe),
        possible_outputs: DF,
        output_name: "out",
        params: &[
            ParamDef {
                name: "path",
                kind: ParamKind::String,
                wireable: true,
                wire_types: SCALAR,
                options: &[],
                default_json: "\"\"",
            },
            ParamDef {
                name: "table",
                kind: ParamKind::String,
                wireable: false,
                wire_types: &[],
                options: &[],
                default_json: "\"\"",
            },
        ],
        can_add_input: false,
        inputs_are_user: false,
    },
    FeatureDef {
        id: "head",
        title: "Head",
        group: "Select",
        inputs: &[IN_DF_OR_SERIES],
        output: None,
        possible_outputs: DF_OR_SERIES,
        output_name: "out",
        params: HEAD_PARAMS,
        can_add_input: false,
        inputs_are_user: false,
    },
    FeatureDef {
        id: "change_headers",
        title: "Change headers",
        group: "Labels",
        inputs: &[IN_DF],
        output: Some(TypeName::Dataframe),
        possible_outputs: DF,
        output_name: "out",
        params: CHANGE_HEADERS_PARAMS,
        can_add_input: false,
        inputs_are_user: false,
    },
    FeatureDef {
        id: "query",
        title: "Query",
        group: "Select",
        inputs: &[IN_DF],
        output: Some(TypeName::Dataframe),
        possible_outputs: DF,
        output_name: "out",
        params: QUERY_PARAMS,
        can_add_input: false,
        inputs_are_user: false,
    },
    FeatureDef {
        id: "column",
        title: "Column",
        group: "Select",
        inputs: &[IN_DF],
        output: Some(TypeName::Series),
        possible_outputs: SERIES,
        output_name: "out",
        params: COLUMN_PARAMS,
        can_add_input: false,
        inputs_are_user: false,
    },
    FeatureDef {
        id: "to_frame",
        title: "To frame",
        group: "Shape",
        inputs: &[PortDef {
            name: "in",
            accepts: SERIES,
            multi: false,
        }],
        output: Some(TypeName::Dataframe),
        possible_outputs: DF,
        output_name: "out",
        params: &[],
        can_add_input: false,
        inputs_are_user: false,
    },
    FeatureDef {
        id: "sort_values",
        title: "Sort",
        group: "Shape",
        inputs: &[IN_DF_OR_SERIES],
        output: None,
        possible_outputs: DF_OR_SERIES,
        output_name: "out",
        params: SORT_PARAMS,
        can_add_input: false,
        inputs_are_user: false,
    },
    FeatureDef {
        id: "merge",
        title: "Merge",
        group: "Shape",
        inputs: &[
            PortDef {
                name: "left",
                accepts: DF,
                multi: false,
            },
            PortDef {
                name: "right",
                accepts: DF,
                multi: false,
            },
        ],
        output: Some(TypeName::Dataframe),
        possible_outputs: DF,
        output_name: "out",
        params: MERGE_PARAMS,
        can_add_input: true,
        inputs_are_user: false,
    },
    FeatureDef {
        id: "concat",
        title: "Concat",
        group: "Shape",
        inputs: &[IN_MULTI],
        output: None,
        possible_outputs: DF_OR_SERIES,
        output_name: "out",
        params: &[],
        can_add_input: true,
        inputs_are_user: false,
    },
    FeatureDef {
        id: "literal",
        title: "Literal",
        group: "Code",
        inputs: &[],
        output: None,
        possible_outputs: &[
            TypeName::Scalar,
            TypeName::List,
            TypeName::Dict,
            TypeName::Code,
        ],
        output_name: "out",
        params: LITERAL_PARAMS,
        can_add_input: false,
        inputs_are_user: false,
    },
    FeatureDef {
        id: "python",
        title: "Python",
        group: "Code",
        inputs: &[],
        output: None,
        possible_outputs: DF,
        output_name: "out",
        params: PYTHON_PARAMS,
        can_add_input: false,
        inputs_are_user: true,
    },
    FeatureDef {
        id: "chart",
        title: "Chart",
        group: "Chart",
        inputs: &[IN_DF],
        output: None,
        possible_outputs: &[],
        output_name: "out",
        params: CHART_PARAMS,
        can_add_input: false,
        inputs_are_user: false,
    },
];

pub fn feature(id: &str) -> Option<&'static FeatureDef> {
    CATALOG.iter().find(|feature| feature.id == id)
}

pub fn features_in(group: &str) -> impl Iterator<Item = &'static FeatureDef> {
    CATALOG.iter().filter(move |feature| feature.group == group)
}

pub fn groups() -> &'static [&'static str] {
    &["IO", "Labels", "Select", "Shape", "Code", "Chart"]
}
