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

const READ_CSV_PARAMS: &[ParamDef] = &[
    ParamDef {
        name: "path",
        kind: ParamKind::String,
        wireable: true,
        wire_types: &[TypeName::Scalar],
        options: &[],
        default_json: "\"\"",
    },
    ParamDef {
        name: "skip_rows",
        kind: ParamKind::Int,
        wireable: false,
        wire_types: &[],
        options: &[],
        default_json: "0",
    },
    ParamDef {
        name: "header",
        kind: ParamKind::Enum,
        wireable: false,
        wire_types: &[],
        options: &["header row", "no header"],
        default_json: "\"header row\"",
    },
    ParamDef {
        name: "header_row",
        kind: ParamKind::Int,
        wireable: false,
        wire_types: &[],
        options: &[],
        default_json: "0",
    },
];

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
        params: EXCEL_PARAMS,
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
            ParamDef {
                name: "skip_rows",
                kind: ParamKind::Int,
                wireable: false,
                wire_types: &[],
                options: &[],
                default_json: "0",
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
        title: "Plot",
        group: "Output",
        inputs: &[IN_DF],
        output: None,
        possible_outputs: &[],
        output_name: "out",
        params: CHART_PARAMS,
        can_add_input: false,
        inputs_are_user: false,
    },
    feature_row(
        "read_json",
        "Read JSON",
        "IO",
        &[],
        Some(TypeName::Dataframe),
        DF,
        PATH_PARAM,
    ),
    feature_row(
        "preview_table",
        "Preview table",
        "Output",
        &[IN_DF],
        None,
        &[],
        &[],
    ),
    feature_row(
        "tail",
        "Tail",
        "Select",
        &[IN_DF_OR_SERIES],
        None,
        DF_OR_SERIES,
        N_PARAM,
    ),
    feature_row(
        "sample",
        "Sample",
        "Select",
        &[IN_DF_OR_SERIES],
        None,
        DF_OR_SERIES,
        N_PARAM,
    ),
    feature_row(
        "drop",
        "Drop",
        "Select",
        &[IN_DF],
        Some(TypeName::Dataframe),
        DF,
        COLUMNS_PARAM,
    ),
    feature_row(
        "filter",
        "Filter",
        "Select",
        &[IN_DF],
        Some(TypeName::Dataframe),
        DF,
        COLUMNS_PARAM,
    ),
    feature_row(
        "drop_duplicates",
        "Drop duplicates",
        "Select",
        &[IN_DF_OR_SERIES],
        None,
        DF_OR_SERIES,
        OPTIONAL_COLUMN,
    ),
    feature_row(
        "nlargest",
        "N largest",
        "Select",
        &[IN_DF],
        Some(TypeName::Dataframe),
        DF,
        N_AND_COLUMN,
    ),
    feature_row(
        "nsmallest",
        "N smallest",
        "Select",
        &[IN_DF],
        Some(TypeName::Dataframe),
        DF,
        N_AND_COLUMN,
    ),
    feature_row(
        "dropna",
        "Drop empty",
        "Clean",
        &[IN_DF_OR_SERIES],
        None,
        DF_OR_SERIES,
        &[],
    ),
    feature_row(
        "fillna",
        "Fill empty",
        "Clean",
        &[IN_DF_OR_SERIES],
        None,
        DF_OR_SERIES,
        VALUE_PARAM,
    ),
    feature_row(
        "replace",
        "Replace",
        "Clean",
        &[IN_DF_OR_SERIES],
        None,
        DF_OR_SERIES,
        REPLACE_PARAM,
    ),
    feature_row(
        "astype",
        "As type",
        "Clean",
        &[IN_DF],
        Some(TypeName::Dataframe),
        DF,
        ASTYPE_PARAM,
    ),
    feature_row(
        "reset_index",
        "Reset index",
        "Shape",
        &[IN_DF_OR_SERIES],
        None,
        DF_OR_SERIES,
        &[],
    ),
    feature_row(
        "set_index",
        "Set index",
        "Shape",
        &[IN_DF],
        Some(TypeName::Dataframe),
        DF,
        COLUMN_PARAM,
    ),
    feature_row(
        "assign",
        "Assign",
        "Shape",
        &[IN_DF],
        Some(TypeName::Dataframe),
        DF,
        ASSIGN_PARAM,
    ),
    feature_row(
        "melt",
        "Melt",
        "Shape",
        &[IN_DF],
        Some(TypeName::Dataframe),
        DF,
        MELT_PARAM,
    ),
    feature_row(
        "pivot",
        "Pivot",
        "Shape",
        &[IN_DF],
        Some(TypeName::Dataframe),
        DF,
        PIVOT_PARAM,
    ),
    feature_row(
        "groupby",
        "Group by",
        "Shape",
        &[IN_DF],
        Some(TypeName::Dataframe),
        DF,
        GROUP_PARAM,
    ),
    feature_row(
        "describe",
        "Describe",
        "Reduce",
        &[IN_DF],
        Some(TypeName::Dataframe),
        DF,
        &[],
    ),
    feature_row(
        "value_counts",
        "Value counts",
        "Reduce",
        &[IN_DF],
        Some(TypeName::Series),
        SERIES,
        COLUMN_PARAM,
    ),
    feature_row(
        "sum",
        "Sum",
        "Reduce",
        &[IN_DF_OR_SERIES],
        Some(TypeName::Series),
        SERIES,
        &[],
    ),
    feature_row(
        "mean",
        "Mean",
        "Reduce",
        &[IN_DF_OR_SERIES],
        Some(TypeName::Series),
        SERIES,
        &[],
    ),
    feature_row(
        "min",
        "Min",
        "Reduce",
        &[IN_DF_OR_SERIES],
        Some(TypeName::Series),
        SERIES,
        &[],
    ),
    feature_row(
        "max",
        "Max",
        "Reduce",
        &[IN_DF_OR_SERIES],
        Some(TypeName::Series),
        SERIES,
        &[],
    ),
    feature_row(
        "count",
        "Count",
        "Reduce",
        &[IN_DF_OR_SERIES],
        Some(TypeName::Series),
        SERIES,
        &[],
    ),
    feature_row(
        "abs",
        "Absolute",
        "Reduce",
        &[IN_DF_OR_SERIES],
        None,
        DF_OR_SERIES,
        &[],
    ),
    feature_row(
        "round",
        "Round",
        "Reduce",
        &[IN_DF_OR_SERIES],
        None,
        DF_OR_SERIES,
        DECIMALS_PARAM,
    ),
    feature_row(
        "median",
        "Median",
        "Reduce",
        &[IN_DF_OR_SERIES],
        Some(TypeName::Series),
        SERIES,
        &[],
    ),
    feature_row(
        "std",
        "Std",
        "Reduce",
        &[IN_DF_OR_SERIES],
        Some(TypeName::Series),
        SERIES,
        &[],
    ),
    feature_row(
        "nunique",
        "N unique",
        "Reduce",
        &[IN_DF_OR_SERIES],
        Some(TypeName::Series),
        SERIES,
        &[],
    ),
    feature_row(
        "str",
        "Text",
        "Text",
        &[IN_DF_OR_SERIES],
        None,
        DF_OR_SERIES,
        STR_PARAM,
    ),
    feature_row(
        "to_datetime",
        "To datetime",
        "Text",
        &[IN_DF_OR_SERIES],
        None,
        DF_OR_SERIES,
        DATETIME_PARAM,
    ),
    feature_row(
        "to_numeric",
        "To numeric",
        "Text",
        &[IN_DF_OR_SERIES],
        None,
        DF_OR_SERIES,
        COLUMN_OPTIONAL,
    ),
    feature_row(
        "dt",
        "Datetime",
        "Text",
        &[IN_DF_OR_SERIES],
        None,
        DF_OR_SERIES,
        DT_PARAM,
    ),
    feature_row(
        "shift",
        "Shift",
        "Shape",
        &[IN_DF_OR_SERIES],
        None,
        DF_OR_SERIES,
        N_PARAM,
    ),
    feature_row(
        "isin",
        "Is in",
        "Select",
        &[IN_DF_OR_SERIES],
        None,
        DF_OR_SERIES,
        ISIN_PARAM,
    ),
    feature_row(
        "between",
        "Between",
        "Select",
        &[IN_DF_OR_SERIES],
        None,
        DF_OR_SERIES,
        BETWEEN_PARAM,
    ),
    feature_row(
        "loc",
        "Loc",
        "Select",
        &[IN_DF_OR_SERIES],
        None,
        DF_OR_SERIES,
        LOC_PARAM,
    ),
    feature_row(
        "iloc",
        "Iloc",
        "Select",
        &[IN_DF_OR_SERIES],
        None,
        DF_OR_SERIES,
        LOC_PARAM,
    ),
    feature_row(
        "ffill",
        "Fill forward",
        "Clean",
        &[IN_DF_OR_SERIES],
        None,
        DF_OR_SERIES,
        LIMIT_PARAM,
    ),
    feature_row(
        "bfill",
        "Fill back",
        "Clean",
        &[IN_DF_OR_SERIES],
        None,
        DF_OR_SERIES,
        LIMIT_PARAM,
    ),
    feature_row(
        "interpolate",
        "Interpolate",
        "Clean",
        &[IN_DF_OR_SERIES],
        None,
        DF_OR_SERIES,
        INTERPOLATE_PARAM,
    ),
    feature_row(
        "clip",
        "Clip",
        "Clean",
        &[IN_DF_OR_SERIES],
        None,
        DF_OR_SERIES,
        CLIP_PARAM,
    ),
    feature_row(
        "where",
        "Where",
        "Clean",
        &[IN_DF_OR_SERIES],
        None,
        DF_OR_SERIES,
        WHERE_PARAM,
    ),
    feature_row(
        "explode",
        "Explode",
        "Shape",
        &[IN_DF],
        Some(TypeName::Dataframe),
        DF,
        COLUMN_PARAM,
    ),
    feature_row(
        "transpose",
        "Transpose",
        "Shape",
        &[IN_DF],
        Some(TypeName::Dataframe),
        DF,
        &[],
    ),
    feature_row(
        "stack",
        "Stack",
        "Shape",
        &[IN_DF],
        Some(TypeName::Series),
        SERIES,
        &[],
    ),
    feature_row(
        "unstack",
        "Unstack",
        "Shape",
        &[IN_DF_OR_SERIES],
        Some(TypeName::Dataframe),
        DF,
        &[],
    ),
    feature_row(
        "crosstab",
        "Crosstab",
        "Shape",
        &[IN_DF],
        Some(TypeName::Dataframe),
        DF,
        CROSSTAB_PARAM,
    ),
    feature_row(
        "rolling",
        "Rolling",
        "Reduce",
        &[IN_DF_OR_SERIES],
        None,
        DF_OR_SERIES,
        ROLLING_PARAM,
    ),
    feature_row(
        "diff",
        "Diff",
        "Reduce",
        &[IN_DF_OR_SERIES],
        None,
        DF_OR_SERIES,
        PERIODS_PARAM,
    ),
    feature_row(
        "pct_change",
        "Percent change",
        "Reduce",
        &[IN_DF_OR_SERIES],
        None,
        DF_OR_SERIES,
        PERIODS_PARAM,
    ),
    feature_row(
        "cumsum",
        "Cumulative sum",
        "Reduce",
        &[IN_DF_OR_SERIES],
        None,
        DF_OR_SERIES,
        &[],
    ),
    feature_row(
        "to_csv",
        "To CSV",
        "Output",
        &[IN_DF_OR_SERIES],
        None,
        &[],
        WRITE_CSV_PARAM,
    ),
    feature_row(
        "to_excel",
        "To Excel",
        "Output",
        &[IN_DF_OR_SERIES],
        None,
        &[],
        WRITE_EXCEL_PARAM,
    ),
    feature_row(
        "to_json",
        "To JSON",
        "Output",
        &[IN_DF_OR_SERIES],
        None,
        &[],
        WRITE_JSON_PARAM,
    ),
    feature_row(
        "to_parquet",
        "To Parquet",
        "Output",
        &[IN_DF_OR_SERIES],
        None,
        &[],
        WRITE_PARQUET_PARAM,
    ),
    feature_row(
        "to_sql",
        "To SQLite",
        "Output",
        &[IN_DF_OR_SERIES],
        None,
        &[],
        WRITE_SQL_PARAM,
    ),
    feature_row(
        "resample",
        "Resample",
        "Time",
        &[IN_DF_OR_SERIES],
        Some(TypeName::Dataframe),
        DF,
        RESAMPLE_PARAM,
    ),
    feature_row(
        "asfreq",
        "As freq",
        "Time",
        &[IN_DF_OR_SERIES],
        Some(TypeName::Dataframe),
        DF,
        ASFREQ_PARAM,
    ),
    feature_row(
        "expanding",
        "Expanding",
        "Time",
        &[IN_DF_OR_SERIES],
        None,
        DF_OR_SERIES,
        ROLLING_PARAM,
    ),
    feature_row(
        "ewm",
        "Ewm",
        "Time",
        &[IN_DF_OR_SERIES],
        None,
        DF_OR_SERIES,
        EWM_PARAM,
    ),
    feature_row(
        "date_range",
        "Date range",
        "Time",
        &[],
        Some(TypeName::Index),
        &[TypeName::Index],
        DATE_RANGE_PARAM,
    ),
    feature_row(
        "tz_localize",
        "Localize zone",
        "Time",
        &[IN_DF_OR_SERIES],
        None,
        DF_OR_SERIES,
        TZ_PARAM,
    ),
    feature_row(
        "tz_convert",
        "Convert zone",
        "Time",
        &[IN_DF_OR_SERIES],
        None,
        DF_OR_SERIES,
        TZ_PARAM,
    ),
    feature_row(
        "to_timedelta",
        "To timedelta",
        "Time",
        &[IN_DF_OR_SERIES],
        None,
        DF_OR_SERIES,
        TIMEDELTA_PARAM,
    ),
    feature_row(
        "cut",
        "Cut",
        "Shape",
        &[IN_DF_OR_SERIES],
        None,
        DF_OR_SERIES,
        CUT_PARAM,
    ),
    feature_row(
        "qcut",
        "Qcut",
        "Shape",
        &[IN_DF_OR_SERIES],
        None,
        DF_OR_SERIES,
        QCUT_PARAM,
    ),
    feature_row(
        "get_dummies",
        "Dummies",
        "Shape",
        &[IN_DF_OR_SERIES],
        Some(TypeName::Dataframe),
        DF,
        DUMMIES_PARAM,
    ),
    feature_row(
        "factorize",
        "Factorize",
        "Shape",
        &[IN_DF_OR_SERIES],
        None,
        DF_OR_SERIES,
        COLUMN_OPTIONAL,
    ),
    feature_row(
        "rank",
        "Rank",
        "Reduce",
        &[IN_DF_OR_SERIES],
        None,
        DF_OR_SERIES,
        RANK_PARAM,
    ),
    feature_row(
        "unique",
        "Unique",
        "Reduce",
        &[IN_DF_OR_SERIES],
        Some(TypeName::Series),
        SERIES,
        COLUMN_OPTIONAL,
    ),
    feature_row(
        "quantile",
        "Quantile",
        "Reduce",
        &[IN_DF_OR_SERIES],
        Some(TypeName::Series),
        SERIES,
        Q_PARAM,
    ),
    feature_row(
        "reindex",
        "Reindex",
        "Shape",
        &[IN_DF_OR_SERIES],
        None,
        DF_OR_SERIES,
        REINDEX_PARAM,
    ),
    feature_row(
        "sort_index",
        "Sort index",
        "Shape",
        &[IN_DF_OR_SERIES],
        None,
        DF_OR_SERIES,
        SORT_INDEX_PARAM,
    ),
    feature_row(
        "wide_to_long",
        "Wide to long",
        "Shape",
        &[IN_DF],
        Some(TypeName::Dataframe),
        DF,
        WIDE_PARAM,
    ),
    feature_row(
        "read_parquet",
        "Read Parquet",
        "IO",
        &[],
        Some(TypeName::Dataframe),
        DF,
        PATH_PARAM,
    ),
    feature_row(
        "read_table",
        "Read table",
        "IO",
        &[],
        Some(TypeName::Dataframe),
        DF,
        READ_TABLE_PARAM,
    ),
    feature_row(
        "read_fwf",
        "Read fixed width",
        "IO",
        &[],
        Some(TypeName::Dataframe),
        DF,
        READ_FWF_PARAM,
    ),
    feature_row(
        "read_html",
        "Read HTML",
        "IO",
        &[],
        Some(TypeName::Dataframe),
        DF,
        READ_HTML_PARAM,
    ),
    feature_row(
        "read_xml",
        "Read XML",
        "IO",
        &[],
        Some(TypeName::Dataframe),
        DF,
        READ_XML_PARAM,
    ),
    feature_row(
        "json_normalize",
        "Normalize JSON",
        "IO",
        &[],
        Some(TypeName::Dataframe),
        DF,
        JSON_NORM_PARAM,
    ),
    feature_row(
        "apply",
        "Apply",
        "Code",
        &[IN_DF_OR_SERIES],
        None,
        DF_OR_SERIES,
        APPLY_PARAM,
    ),
    feature_row(
        "map",
        "Map",
        "Code",
        &[IN_DF_OR_SERIES],
        None,
        DF_OR_SERIES,
        MAP_PARAM,
    ),
    FeatureDef {
        id: "arith",
        title: "Arithmetic",
        group: "Code",
        inputs: &[IN_DF_OR_SERIES, IN_OTHER],
        output: None,
        possible_outputs: DF_OR_SERIES,
        output_name: "out",
        params: ARITH_PARAM,
        can_add_input: false,
        inputs_are_user: false,
    },
    FeatureDef {
        id: "corr",
        title: "Correlation",
        group: "Reduce",
        inputs: &[IN_DF_OR_SERIES, IN_OTHER_SERIES],
        output: None,
        possible_outputs: DF_OR_SERIES,
        output_name: "out",
        params: &[],
        can_add_input: false,
        inputs_are_user: false,
    },
    FeatureDef {
        id: "cov",
        title: "Covariance",
        group: "Reduce",
        inputs: &[IN_DF_OR_SERIES, IN_OTHER_SERIES],
        output: None,
        possible_outputs: DF_OR_SERIES,
        output_name: "out",
        params: &[],
        can_add_input: false,
        inputs_are_user: false,
    },
    FeatureDef {
        id: "combine_first",
        title: "Combine first",
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
        params: &[],
        can_add_input: false,
        inputs_are_user: false,
    },
    FeatureDef {
        id: "join",
        title: "Join",
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
        params: JOIN_PARAM,
        can_add_input: false,
        inputs_are_user: false,
    },
    FeatureDef {
        id: "merge_asof",
        title: "Merge asof",
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
        params: ASOF_PARAM,
        can_add_input: false,
        inputs_are_user: false,
    },
];

const EXCEL_PARAMS: &[ParamDef] = &[
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
    ParamDef {
        name: "skip_rows",
        kind: ParamKind::Int,
        wireable: false,
        wire_types: &[],
        options: &[],
        default_json: "0",
    },
    ParamDef {
        name: "header",
        kind: ParamKind::Enum,
        wireable: false,
        wire_types: &[],
        options: &["header row", "no header"],
        default_json: "\"header row\"",
    },
    ParamDef {
        name: "header_row",
        kind: ParamKind::Int,
        wireable: false,
        wire_types: &[],
        options: &[],
        default_json: "0",
    },
];

const PATH_PARAM: &[ParamDef] = &[ParamDef {
    name: "path",
    kind: ParamKind::String,
    wireable: true,
    wire_types: SCALAR,
    options: &[],
    default_json: "\"\"",
}];
const N_PARAM: &[ParamDef] = &[ParamDef {
    name: "n",
    kind: ParamKind::Int,
    wireable: true,
    wire_types: SCALAR,
    options: &[],
    default_json: "5",
}];
const COLUMNS_PARAM: &[ParamDef] = &[ParamDef {
    name: "columns",
    kind: ParamKind::String,
    wireable: false,
    wire_types: &[],
    options: &[],
    default_json: "\"\"",
}];
const COLUMN_PARAM: &[ParamDef] = &[ParamDef {
    name: "column",
    kind: ParamKind::Column,
    wireable: false,
    wire_types: &[],
    options: &[],
    default_json: "\"\"",
}];
const OPTIONAL_COLUMN: &[ParamDef] = &[ParamDef {
    name: "column",
    kind: ParamKind::Column,
    wireable: false,
    wire_types: &[],
    options: &[],
    default_json: "\"\"",
}];
const VALUE_PARAM: &[ParamDef] = &[ParamDef {
    name: "value",
    kind: ParamKind::String,
    wireable: false,
    wire_types: &[],
    options: &[],
    default_json: "\"\"",
}];
const REPLACE_PARAM: &[ParamDef] = &[
    ParamDef {
        name: "old",
        kind: ParamKind::String,
        wireable: false,
        wire_types: &[],
        options: &[],
        default_json: "\"\"",
    },
    ParamDef {
        name: "new",
        kind: ParamKind::String,
        wireable: false,
        wire_types: &[],
        options: &[],
        default_json: "\"\"",
    },
];
const ASTYPE_PARAM: &[ParamDef] = &[
    COLUMN_PARAM[0],
    ParamDef {
        name: "dtype",
        kind: ParamKind::Enum,
        wireable: false,
        wire_types: &[],
        options: &["float", "int", "string", "bool"],
        default_json: "\"float\"",
    },
];
const N_AND_COLUMN: &[ParamDef] = &[N_PARAM[0], COLUMN_PARAM[0]];
const ASSIGN_PARAM: &[ParamDef] = &[
    ParamDef {
        name: "name",
        kind: ParamKind::String,
        wireable: false,
        wire_types: &[],
        options: &[],
        default_json: "\"\"",
    },
    ParamDef {
        name: "expr",
        kind: ParamKind::String,
        wireable: false,
        wire_types: &[],
        options: &[],
        default_json: "\"\"",
    },
];
const MELT_PARAM: &[ParamDef] = &[ParamDef {
    name: "id_vars",
    kind: ParamKind::String,
    wireable: false,
    wire_types: &[],
    options: &[],
    default_json: "\"\"",
}];
const PIVOT_PARAM: &[ParamDef] = &[
    ParamDef {
        name: "index",
        kind: ParamKind::Column,
        wireable: false,
        wire_types: &[],
        options: &[],
        default_json: "\"\"",
    },
    ParamDef {
        name: "columns",
        kind: ParamKind::Column,
        wireable: false,
        wire_types: &[],
        options: &[],
        default_json: "\"\"",
    },
    ParamDef {
        name: "values",
        kind: ParamKind::Column,
        wireable: false,
        wire_types: &[],
        options: &[],
        default_json: "\"\"",
    },
    ParamDef {
        name: "agg",
        kind: ParamKind::Enum,
        wireable: false,
        wire_types: &[],
        options: &["sum", "mean", "count", "min", "max"],
        default_json: "\"sum\"",
    },
];
const GROUP_PARAM: &[ParamDef] = &[
    ParamDef {
        name: "by",
        kind: ParamKind::Column,
        wireable: false,
        wire_types: &[],
        options: &[],
        default_json: "\"\"",
    },
    ParamDef {
        name: "agg",
        kind: ParamKind::Enum,
        wireable: false,
        wire_types: &[],
        options: &[
            "sum", "mean", "count", "min", "max", "median", "std", "nunique", "quantile",
        ],
        default_json: "\"sum\"",
    },
    ParamDef {
        name: "how",
        kind: ParamKind::Enum,
        wireable: false,
        wire_types: &[],
        options: &["agg", "transform", "filter"],
        default_json: "\"agg\"",
    },
    COLUMN_OPTIONAL[0],
    ParamDef {
        name: "filter_op",
        kind: ParamKind::Enum,
        wireable: false,
        wire_types: &[],
        options: &["gt", "ge", "lt", "le", "eq"],
        default_json: "\"gt\"",
    },
    text_param("filter_value"),
    ParamDef {
        name: "q",
        kind: ParamKind::Float,
        wireable: false,
        wire_types: &[],
        options: &[],
        default_json: "0.5",
    },
    ParamDef {
        name: "aggs",
        kind: ParamKind::String,
        wireable: false,
        wire_types: &[],
        options: &[],
        default_json: "\"\"",
    },
];
const DECIMALS_PARAM: &[ParamDef] = &[ParamDef {
    name: "decimals",
    kind: ParamKind::Int,
    wireable: false,
    wire_types: &[],
    options: &[],
    default_json: "2",
}];

const STR_METHODS: &[&str] = &[
    "lower",
    "upper",
    "title",
    "strip",
    "lstrip",
    "rstrip",
    "len",
    "contains",
    "replace",
    "slice",
    "split",
    "rsplit",
    "extract",
    "startswith",
    "endswith",
    "cat",
    "get",
    "zfill",
    "pad",
    "count",
    "find",
    "match",
    "isalnum",
    "isalpha",
    "isdigit",
    "isnumeric",
];
const DT_ACCESSORS: &[&str] = &[
    "year",
    "month",
    "day",
    "hour",
    "minute",
    "second",
    "dayofweek",
    "day_name",
    "quarter",
    "date",
    "floor",
    "ceil",
    "round",
    "strftime",
];

const fn text_param(name: &'static str) -> ParamDef {
    ParamDef {
        name,
        kind: ParamKind::String,
        wireable: false,
        wire_types: &[],
        options: &[],
        default_json: "\"\"",
    }
}

const fn int_param(name: &'static str, default_json: &'static str) -> ParamDef {
    ParamDef {
        name,
        kind: ParamKind::Int,
        wireable: false,
        wire_types: &[],
        options: &[],
        default_json,
    }
}

const COLUMN_OPTIONAL: &[ParamDef] = &[ParamDef {
    name: "column",
    kind: ParamKind::Column,
    wireable: false,
    wire_types: &[],
    options: &[],
    default_json: "\"\"",
}];

const STR_PARAM: &[ParamDef] = &[
    ParamDef {
        name: "method",
        kind: ParamKind::Enum,
        wireable: false,
        wire_types: &[],
        options: STR_METHODS,
        default_json: "\"lower\"",
    },
    COLUMN_OPTIONAL[0],
    text_param("pat"),
    text_param("repl"),
    int_param("start", "\"\""),
    int_param("stop", "\"\""),
    int_param("n", "-1"),
    int_param("width", "\"\""),
    ParamDef {
        name: "side",
        kind: ParamKind::Enum,
        wireable: false,
        wire_types: &[],
        options: &["left", "right", "both"],
        default_json: "\"left\"",
    },
    text_param("fill"),
    text_param("sep"),
];

const DATETIME_PARAM: &[ParamDef] = &[COLUMN_OPTIONAL[0], text_param("format")];

const DT_PARAM: &[ParamDef] = &[
    ParamDef {
        name: "accessor",
        kind: ParamKind::Enum,
        wireable: false,
        wire_types: &[],
        options: DT_ACCESSORS,
        default_json: "\"year\"",
    },
    COLUMN_OPTIONAL[0],
    text_param("freq"),
    text_param("fmt"),
];

const ISIN_PARAM: &[ParamDef] = &[COLUMN_OPTIONAL[0], text_param("values")];

const BETWEEN_PARAM: &[ParamDef] = &[
    COLUMN_OPTIONAL[0],
    text_param("lower"),
    text_param("upper"),
    ParamDef {
        name: "inclusive",
        kind: ParamKind::Enum,
        wireable: false,
        wire_types: &[],
        options: &["both", "neither", "left", "right"],
        default_json: "\"both\"",
    },
];

const LOC_PARAM: &[ParamDef] = &[text_param("rows"), text_param("columns")];

const LIMIT_PARAM: &[ParamDef] = &[int_param("limit", "\"\"")];

const INTERPOLATE_PARAM: &[ParamDef] = &[
    ParamDef {
        name: "method",
        kind: ParamKind::Enum,
        wireable: false,
        wire_types: &[],
        options: &["linear", "nearest", "zero"],
        default_json: "\"linear\"",
    },
    int_param("limit", "\"\""),
];

const CLIP_PARAM: &[ParamDef] = &[text_param("lower"), text_param("upper")];

const WHERE_PARAM: &[ParamDef] = &[
    ParamDef {
        name: "mode",
        kind: ParamKind::Enum,
        wireable: false,
        wire_types: &[],
        options: &["where", "mask"],
        default_json: "\"where\"",
    },
    text_param("cond"),
    text_param("other"),
];

const CROSSTAB_PARAM: &[ParamDef] = &[
    ParamDef {
        name: "index",
        kind: ParamKind::Column,
        wireable: false,
        wire_types: &[],
        options: &[],
        default_json: "\"\"",
    },
    ParamDef {
        name: "columns",
        kind: ParamKind::Column,
        wireable: false,
        wire_types: &[],
        options: &[],
        default_json: "\"\"",
    },
    ParamDef {
        name: "values",
        kind: ParamKind::Column,
        wireable: false,
        wire_types: &[],
        options: &[],
        default_json: "\"\"",
    },
    ParamDef {
        name: "agg",
        kind: ParamKind::Enum,
        wireable: false,
        wire_types: &[],
        options: &["count", "sum", "mean", "min", "max"],
        default_json: "\"count\"",
    },
];

const ROLLING_PARAM: &[ParamDef] = &[
    int_param("window", "3"),
    ParamDef {
        name: "agg",
        kind: ParamKind::Enum,
        wireable: false,
        wire_types: &[],
        options: &["mean", "sum", "min", "max", "std", "median", "quantile"],
        default_json: "\"mean\"",
    },
    COLUMN_OPTIONAL[0],
    ParamDef {
        name: "q",
        kind: ParamKind::Float,
        wireable: false,
        wire_types: &[],
        options: &[],
        default_json: "0.5",
    },
];

const PERIODS_PARAM: &[ParamDef] = &[int_param("periods", "1")];

const INDEX_FLAG: ParamDef = ParamDef {
    name: "index",
    kind: ParamKind::Enum,
    wireable: false,
    wire_types: &[],
    options: &["no", "yes"],
    default_json: "\"no\"",
};

const WRITE_CSV_PARAM: &[ParamDef] = &[
    PATH_PARAM[0],
    INDEX_FLAG,
    ParamDef {
        name: "sep",
        kind: ParamKind::String,
        wireable: false,
        wire_types: &[],
        options: &[],
        default_json: "\",\"",
    },
];
const WRITE_EXCEL_PARAM: &[ParamDef] = &[PATH_PARAM[0], text_param("sheet"), INDEX_FLAG];
const WRITE_JSON_PARAM: &[ParamDef] = &[
    PATH_PARAM[0],
    ParamDef {
        name: "orient",
        kind: ParamKind::Enum,
        wireable: false,
        wire_types: &[],
        options: &["records", "columns", "split", "index"],
        default_json: "\"records\"",
    },
];
const WRITE_PARQUET_PARAM: &[ParamDef] = &[PATH_PARAM[0], INDEX_FLAG];
const WRITE_SQL_PARAM: &[ParamDef] = &[
    PATH_PARAM[0],
    text_param("table"),
    ParamDef {
        name: "if_exists",
        kind: ParamKind::Enum,
        wireable: false,
        wire_types: &[],
        options: &["replace", "append", "fail"],
        default_json: "\"replace\"",
    },
    INDEX_FLAG,
];

const JOIN_PARAM: &[ParamDef] = &[
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
        options: &["left", "right", "inner", "outer"],
        default_json: "\"left\"",
    },
];

const ASOF_PARAM: &[ParamDef] = &[
    ParamDef {
        name: "on",
        kind: ParamKind::Column,
        wireable: false,
        wire_types: &[],
        options: &[],
        default_json: "\"\"",
    },
    ParamDef {
        name: "direction",
        kind: ParamKind::Enum,
        wireable: false,
        wire_types: &[],
        options: &["backward", "forward", "nearest"],
        default_json: "\"backward\"",
    },
];

const OTHER_VALUE_TYPES: &[TypeName] = &[TypeName::Dataframe, TypeName::Series, TypeName::Scalar];
const IN_OTHER: PortDef = PortDef {
    name: "other",
    accepts: OTHER_VALUE_TYPES,
    multi: false,
};
const IN_OTHER_SERIES: PortDef = PortDef {
    name: "other",
    accepts: SERIES,
    multi: false,
};

const AGG_NAMES: &[&str] = &[
    "mean", "sum", "min", "max", "std", "median", "count", "nunique", "quantile",
];

const RESAMPLE_PARAM: &[ParamDef] = &[
    text_param("freq"),
    COLUMN_OPTIONAL[0],
    ParamDef {
        name: "agg",
        kind: ParamKind::Enum,
        wireable: false,
        wire_types: &[],
        options: AGG_NAMES,
        default_json: "\"mean\"",
    },
    ParamDef {
        name: "fill",
        kind: ParamKind::Enum,
        wireable: false,
        wire_types: &[],
        options: &["none", "ffill", "bfill"],
        default_json: "\"none\"",
    },
    ParamDef {
        name: "q",
        kind: ParamKind::Float,
        wireable: false,
        wire_types: &[],
        options: &[],
        default_json: "0.5",
    },
];

const ASFREQ_PARAM: &[ParamDef] = &[
    text_param("freq"),
    COLUMN_OPTIONAL[0],
    ParamDef {
        name: "fill",
        kind: ParamKind::Enum,
        wireable: false,
        wire_types: &[],
        options: &["none", "ffill", "bfill"],
        default_json: "\"none\"",
    },
];

const EWM_PARAM: &[ParamDef] = &[
    int_param("span", "3"),
    ParamDef {
        name: "agg",
        kind: ParamKind::Enum,
        wireable: false,
        wire_types: &[],
        options: &["mean", "std"],
        default_json: "\"mean\"",
    },
    COLUMN_OPTIONAL[0],
];

const DATE_RANGE_PARAM: &[ParamDef] = &[
    text_param("start"),
    text_param("end"),
    text_param("freq"),
    int_param("periods", "\"\""),
];

const TZ_PARAM: &[ParamDef] = &[text_param("tz"), COLUMN_OPTIONAL[0]];
const TIMEDELTA_PARAM: &[ParamDef] = &[COLUMN_OPTIONAL[0], text_param("unit")];

const CUT_PARAM: &[ParamDef] = &[COLUMN_OPTIONAL[0], text_param("bins"), text_param("labels")];
const QCUT_PARAM: &[ParamDef] = &[
    COLUMN_OPTIONAL[0],
    int_param("q", "4"),
    text_param("labels"),
];
const DUMMIES_PARAM: &[ParamDef] = &[COLUMN_OPTIONAL[0], text_param("prefix")];
const RANK_PARAM: &[ParamDef] = &[
    COLUMN_OPTIONAL[0],
    ParamDef {
        name: "method",
        kind: ParamKind::Enum,
        wireable: false,
        wire_types: &[],
        options: &["average", "min", "max", "first", "dense"],
        default_json: "\"average\"",
    },
];
const Q_PARAM: &[ParamDef] = &[ParamDef {
    name: "q",
    kind: ParamKind::Float,
    wireable: false,
    wire_types: &[],
    options: &[],
    default_json: "0.5",
}];
const REINDEX_PARAM: &[ParamDef] = &[
    text_param("index"),
    text_param("columns"),
    text_param("fill"),
];
const SORT_INDEX_PARAM: &[ParamDef] = &[
    ParamDef {
        name: "order",
        kind: ParamKind::Enum,
        wireable: false,
        wire_types: &[],
        options: &["ascending", "descending"],
        default_json: "\"ascending\"",
    },
    ParamDef {
        name: "axis",
        kind: ParamKind::Enum,
        wireable: false,
        wire_types: &[],
        options: &["0", "1"],
        default_json: "\"0\"",
    },
];
const WIDE_PARAM: &[ParamDef] = &[
    text_param("stubnames"),
    text_param("i"),
    text_param("j"),
    text_param("sep"),
];
const READ_TABLE_PARAM: &[ParamDef] = &[
    PATH_PARAM[0],
    ParamDef {
        name: "sep",
        kind: ParamKind::String,
        wireable: false,
        wire_types: &[],
        options: &[],
        default_json: "\"\\t\"",
    },
    READ_CSV_PARAMS[1],
    READ_CSV_PARAMS[2],
    READ_CSV_PARAMS[3],
];
const READ_FWF_PARAM: &[ParamDef] = &[
    PATH_PARAM[0],
    text_param("colspecs"),
    READ_CSV_PARAMS[1],
    READ_CSV_PARAMS[2],
    READ_CSV_PARAMS[3],
];
const READ_HTML_PARAM: &[ParamDef] = &[
    PATH_PARAM[0],
    int_param("which", "0"),
    READ_CSV_PARAMS[1],
    READ_CSV_PARAMS[2],
    READ_CSV_PARAMS[3],
];
const READ_XML_PARAM: &[ParamDef] = &[PATH_PARAM[0], text_param("xpath"), READ_CSV_PARAMS[1]];
const JSON_NORM_PARAM: &[ParamDef] = &[
    PATH_PARAM[0],
    text_param("record_path"),
    text_param("meta"),
    text_param("sep"),
];
const APPLY_PARAM: &[ParamDef] = &[
    ParamDef {
        name: "fn",
        kind: ParamKind::Enum,
        wireable: false,
        wire_types: &[],
        options: &[
            "sum", "mean", "min", "max", "count", "median", "std", "quantile", "code",
        ],
        default_json: "\"sum\"",
    },
    ParamDef {
        name: "axis",
        kind: ParamKind::Enum,
        wireable: false,
        wire_types: &[],
        options: &["0", "1"],
        default_json: "\"0\"",
    },
    ParamDef {
        name: "q",
        kind: ParamKind::Float,
        wireable: false,
        wire_types: &[],
        options: &[],
        default_json: "0.5",
    },
    ParamDef {
        name: "code",
        kind: ParamKind::Code,
        wireable: false,
        wire_types: &[],
        options: &[],
        default_json: "\"def run(values):\\n    return values\\n\"",
    },
];
const MAP_PARAM: &[ParamDef] = &[
    ParamDef {
        name: "fn",
        kind: ParamKind::Enum,
        wireable: false,
        wire_types: &[],
        options: &[
            "sum", "mean", "min", "max", "count", "median", "std", "quantile", "code",
        ],
        default_json: "\"code\"",
    },
    ParamDef {
        name: "code",
        kind: ParamKind::Code,
        wireable: false,
        wire_types: &[],
        options: &[],
        default_json: "\"def run(value):\\n    return value\\n\"",
    },
];
const ARITH_PARAM: &[ParamDef] = &[
    ParamDef {
        name: "op",
        kind: ParamKind::Enum,
        wireable: false,
        wire_types: &[],
        options: &[
            "add", "sub", "mul", "div", "floordiv", "mod", "pow", "eq", "ne", "lt", "gt", "le",
            "ge",
        ],
        default_json: "\"add\"",
    },
    text_param("value"),
];

const fn feature_row(
    id: &'static str,
    title: &'static str,
    group: &'static str,
    inputs: &'static [PortDef],
    output: Option<TypeName>,
    possible: &'static [TypeName],
    params: &'static [ParamDef],
) -> FeatureDef {
    FeatureDef {
        id,
        title,
        group,
        inputs,
        output,
        possible_outputs: possible,
        output_name: "out",
        params,
        can_add_input: false,
        inputs_are_user: false,
    }
}

pub fn feature(id: &str) -> Option<&'static FeatureDef> {
    CATALOG.iter().find(|feature| feature.id == id)
}

pub fn features_in(group: &str) -> impl Iterator<Item = &'static FeatureDef> {
    CATALOG.iter().filter(move |feature| feature.group == group)
}

pub fn groups() -> &'static [&'static str] {
    &[
        "IO", "Labels", "Select", "Shape", "Clean", "Text", "Time", "Reduce", "Code", "Output",
    ]
}
