use heck::ToSnakeCase;
use pgorm_query::{Deferrability, Enforcement, ForeignKeyAction, TableForeignKey};

use crate::{Error, TableIdent, util::escape_rust_keyword};

#[derive(Clone, Debug)]
pub enum RelationType {
    HasOne,
    HasMany,
    BelongsTo,
}

#[derive(Clone, Debug)]
pub struct Relation {
    pub(crate) ref_table: String,
    /// The schema qualifying the referenced table, once the relation has been
    /// resolved against the schema's own tables. Never reaches the generated
    /// text: a module path is the bare name, which is one table's alone.
    pub(crate) ref_schema: Option<String>,
    pub(crate) columns: Vec<String>,
    pub(crate) ref_columns: Vec<String>,
    pub(crate) rel_type: RelationType,
    pub(crate) on_update: Option<ForeignKeyAction>,
    pub(crate) on_delete: Option<ForeignKeyAction>,
    /// `NotEnforced` when the key is; an explicit `ENFORCED` states the
    /// default and reads as nothing said.
    pub(crate) enforcement: Option<Enforcement>,
    /// When the key is checked, if it is deferrable; an explicit `NOT
    /// DEFERRABLE` states the default and reads as nothing said.
    pub(crate) deferrability: Option<Deferrability>,
    /// The `(column, referenced column)` pair a temporal foreign key matches
    /// as periods, after its pairs.
    // [spec:pgorm:sem:codegen.entity.relations+5]
    pub(crate) period: Option<(String, String)>,
    pub(crate) self_referencing: bool,
    pub(crate) num_suffix: usize,
    pub(crate) impl_related: bool,
}

impl Relation {
    /// Which table this relation references.
    // [spec:pgorm:sem:codegen.entity.transform+18]
    pub fn ref_ident(&self) -> TableIdent {
        TableIdent {
            table: self.ref_table.clone(),
            schema: self.ref_schema.clone(),
        }
    }

    /// What a writer could not write of the relation: an action codegen has
    /// no spelling of, and columns that do not pair with the referenced ones.
    /// Whether its DB names derive Rust identifiers is the Rust writer's
    /// gate's to check.
    // [spec:pgorm:sem:codegen.entity.relations+5]
    pub(crate) fn validate(&self) -> Result<(), Error> {
        let context = format!("relation to `{}`", self.ref_ident());
        for action in self.on_update.iter().chain(&self.on_delete) {
            if Self::get_foreign_key_action(action).is_none() {
                return Err(Error::TransformError(format!(
                    "{context}: foreign key action {action:?} is not supported by codegen"
                )));
            }
        }
        if self.columns.len() != self.ref_columns.len() {
            return Err(Error::TransformError(format!(
                "{context}: constrains {} column(s) against {} referenced column(s); a relation \
                 joins its columns in pairs",
                self.columns.len(),
                self.ref_columns.len(),
            )));
        }
        Ok(())
    }

    /// The module of the entity the relation reaches, keyword-escaped as the
    /// writer names it, or `None` for a self-reference, which reaches its
    /// own entity.
    pub(crate) fn module_name(&self) -> Option<String> {
        (!self.self_referencing).then(|| escape_rust_keyword(self.ref_table.to_snake_case()))
    }

    /// The `ForeignKeyAction` variant naming `action`, as generated source
    /// spells it, or `None` for an action codegen has no spelling of, which
    /// [`validate`](Self::validate) refuses.
    pub fn get_foreign_key_action(action: &ForeignKeyAction) -> Option<String> {
        let action = match action {
            ForeignKeyAction::Restrict => "Restrict",
            ForeignKeyAction::Cascade => "Cascade",
            ForeignKeyAction::SetNull => "SetNull",
            ForeignKeyAction::NoAction => "NoAction",
            ForeignKeyAction::SetDefault => "SetDefault",
            _ => return None,
        };
        Some(action.to_owned())
    }
}

// [spec:pgorm:sem:codegen.entity.transform+18]
impl From<&TableForeignKey> for Relation {
    fn from(tbl_fk: &TableForeignKey) -> Self {
        let ref_table = TableIdent::of(tbl_fk.get_ref_table());
        Self {
            ref_table: ref_table.table,
            ref_schema: ref_table.schema,
            columns: tbl_fk.get_columns(),
            ref_columns: tbl_fk.get_ref_columns(),
            rel_type: RelationType::BelongsTo,
            on_delete: tbl_fk.get_on_delete(),
            on_update: tbl_fk.get_on_update(),
            enforcement: tbl_fk
                .get_enforcement()
                .filter(|enforcement| *enforcement == Enforcement::NotEnforced),
            deferrability: tbl_fk
                .get_deferrability()
                .filter(|deferrability| *deferrability != Deferrability::NotDeferrable),
            period: tbl_fk
                .get_period()
                .map(|(column, ref_column)| (column.to_string(), ref_column.to_string())),
            self_referencing: false,
            num_suffix: 0,
            impl_related: true,
        }
    }
}
