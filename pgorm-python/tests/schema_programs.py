"""DDL inputs shared by independent native parity and installed-wheel tests."""


def programs(p):
    table = p.Table('items "雪"', schema='schema "雪"')
    kind = p.TypeName('Mood "雪"', schema='schema "雪"')
    id_col = p.ColumnDef('id "x"', "integer").not_null()
    base = p.create_table(table).column(id_col)
    return {
        "table": base.column(p.ColumnDef("name", p.DataType("varchar", length=80)).default("O'Brien \\ 雪"))
            .column(p.ColumnDef("mood", kind).default(p.Value("busy", kind)))
            .column(p.ColumnDef("moods", p.DataType(kind).array()))
            .column(p.ColumnDef("amount", p.DataType("numeric", precision=12, scale=3)))
            .primary_key('id "x"').unique("name", name='unique "x"', nulls_not_distinct=True)
            .check(p.col('id "x"') > 0).if_not_exists(),
        "generated": base.column(p.ColumnDef("twice", "integer").generated(p.col('id "x"') * 2, "stored")),
        "generated_virtual": base.column(p.ColumnDef("next", "integer").generated(p.col('id "x"') + 1, "virtual")),
        "column_specs": p.create_table(table).column(p.ColumnDef("id", "bigint").auto_increment())
            .column(p.ColumnDef("n", "integer").null().check(p.col("n") > 0)).primary_key("id").unique("n"),
        "index": p.create_index(table, "name", name='index "x"').column('id "x"', order="desc")
            .unique().nulls_not_distinct().using("btree").if_not_exists(),
        "index_gin": p.create_index(table, "moods").using("gin"),
        "drop_index": p.drop_index(table, 'index "x"', if_exists=True),
        "drop_table": p.drop_table(table, if_exists=True, behavior="cascade"),
        "rename_table": p.rename_table(table, 'new "x"'),
        "rename_column": p.rename_column(table, 'id "x"', 'new "x"'),
        "truncate_table": p.truncate_table(table),
        "add_column": p.alter_table(table).add_column(p.ColumnDef("extra", "text"), if_not_exists=True),
        "modify_column": p.alter_table(table).modify_column(p.ColumnDef("extra", "varchar").not_null().default("hello")),
        "collated": p.create_table(table).column(p.ColumnDef('word "x"', "text").not_null().collate("C"))
            .column(p.ColumnDef("label", "text").collate("C").collate("POSIX", schema="pg_catalog")),
        "add_column_collated": p.alter_table(table).add_column(p.ColumnDef("extra", "text").collate('C "x"', schema='schema "雪"')),
        "modify_column_collated": p.alter_table(table).modify_column(p.ColumnDef("extra", "varchar").collate("C")),
        "add_primary_key": p.alter_table(table).add_primary_key(['id "x"', "name"]),
        "add_unique": p.alter_table(table).add_unique(("name", 'id "x"'), name='unique "x"', nulls_not_distinct=True),
        "temporal_keys": base.column(p.ColumnDef('during "x"', p.TypeName("tstzrange")))
            .primary_key('id "x"', without_overlaps='during "x"')
            .unique(["name", 'id "x"'], name='unique "x"', nulls_not_distinct=True, without_overlaps='during "x"'),
        "add_primary_key_temporal": p.alter_table(table).add_primary_key('id "x"', without_overlaps='during "x"'),
        "add_unique_temporal": p.alter_table(table).add_unique("name", without_overlaps='during "x"'),
        "drop_column": p.alter_table(table).drop_column("extra"),
        "set_expression": p.alter_table(table).set_expression("twice", p.col('id "x"') * 3),
        "drop_expression": p.alter_table(table).drop_expression("twice"),
        "drop_expression_if_exists": p.alter_table(table).drop_expression("twice", if_exists=True),
        "not_null_named": p.create_table(table).column(p.ColumnDef("extra", "text").not_null(name='present "x"', no_inherit=True))
            .column(p.ColumnDef("kept", "text").not_null(no_inherit=True)),
        "add_not_null": p.alter_table(table).add_not_null("extra"),
        "add_not_null_named": p.alter_table(table).add_not_null("extra", name='present "x"', no_inherit=True, not_valid=True),
        "validate_constraint": p.alter_table(table).validate_constraint('present "x"'),
        "alter_constraint_inherit": p.alter_table(table).alter_constraint('present "x"', "inherit"),
        "alter_constraint_no_inherit": p.alter_table(table).alter_constraint('present "x"', "no_inherit"),
        "drop_constraint": p.alter_table(table).drop_constraint('present "x"'),
        "drop_constraint_if_exists": p.alter_table(table).drop_constraint('present "x"', if_exists=True, behavior="cascade"),
        "rename_constraint": p.rename_constraint(table, 'present "x"', 'kept "x"'),
        "check_named": p.create_table(table)
            .column(p.ColumnDef("n", "integer").check(p.col("n") > 0, name='positive "x"', enforcement="not_enforced"))
            .check(p.col("n") < 100, name='small "x"').check(p.col("n") != 7, enforcement="not_enforced"),
        "add_check": p.alter_table(table).add_check(p.col("n") > 0, name='positive "x"', enforcement="not_enforced"),
        "add_check_plain": p.alter_table(table).add_check(p.col("n") > 0),
        "add_check_not_valid": p.alter_table(table).add_check(p.col("n") > 0, name='positive "x"', no_inherit=True, not_valid=True),
        "check_no_inherit": p.create_table(table)
            .column(p.ColumnDef("n", "integer").check(p.col("n") > 0, no_inherit=True))
            .check(p.col("n") < 100, name='small "x"', no_inherit=True, enforcement="not_enforced"),
        "alter_constraint_enforced": p.alter_table(table).alter_constraint('fk "x"', "enforced"),
        "alter_constraint_not_enforced": p.alter_table(table).alter_constraint('fk "x"', "not_enforced"),
        "enum": p.create_type(kind).values(["", "O'Brien \\ 雪", "busy"]),
        "enum_before": p.alter_type(kind).add_value("new", before="busy"),
        "enum_after": p.alter_type(kind).add_value("new", after="busy"),
        "enum_rename_value": p.alter_type(kind).rename_value("busy", "calm"),
        "enum_rename": p.alter_type(kind).rename_to('new "x"'),
        "enum_drop": p.drop_type(kind, if_exists=True, behavior="cascade"),
        **views(p, table),
        **schemas(p),
        **domains(p),
    }


def domains(p):
    name = p.TypeName('Qty "雪"', schema='schema "雪"')
    value = p.domain_value()
    return {
        "create_domain": p.create_domain('qty "雪"', "integer"),
        "create_domain_clauses": p.create_domain(name, "text")
            .check(value != "", name='filled "x"').not_null().default("O'Brien \\ 雪")
            .collate("C", schema="pg_catalog").check(value != "none"),
        "create_domain_over_array": p.create_domain(name, p.DataType("varchar", length=20).array())
            .null().default(p.literal(1) + 2),
        "create_domain_over_domain": p.create_domain("small", name).not_null().null(),
        "set_domain_default": p.alter_domain(name).set_default(5),
        "drop_domain_default": p.alter_domain(name).drop_default(),
        "set_domain_not_null": p.alter_domain(name).set_not_null(),
        "drop_domain_not_null": p.alter_domain(name).drop_not_null(),
        "add_domain_check": p.alter_domain(name).add_check(value > 0),
        "add_domain_check_not_valid": p.alter_domain(name).add_check(
            value < 1000, name='small "x"', not_valid=True,
        ),
        "drop_domain_constraint": p.alter_domain(name).drop_constraint(
            'small "x"', if_exists=True, behavior="cascade",
        ),
        "rename_domain_constraint": p.alter_domain(name).rename_constraint('small "x"', 'tiny "雪"'),
        "validate_domain_constraint": p.alter_domain(name).validate_constraint('small "x"'),
        "rename_domain": p.alter_domain(name).rename_to('units "雪"'),
        "drop_domain": p.drop_domain(name),
        "drop_domains": p.drop_domain(
            [name, "plain", p.TypeName("other", schema="archive")], if_exists=True, behavior="cascade",
        ),
    }


def schemas(p):
    name = 'schema "雪"'
    return {
        "create_schema": p.create_schema(name),
        "create_schema_if_not_exists": p.create_schema(name, if_not_exists=True),
        "drop_schema": p.drop_schema(name),
        "drop_schemas": p.drop_schema([name, "archive", 'Old "x"'], if_exists=True, behavior="cascade"),
        "rename_schema": p.rename_schema(name, 'renamed "雪"'),
    }


def views(p, table):
    query = p.Select(p.col('id "x"'), p.col("name")).from_(table).where(p.col("name") != "O'Brien \\ 雪")
    view = p.Table('view "雪"', schema='schema "雪"')
    return {
        "create_view": p.create_view(view, query),
        "create_view_options": p.create_view(view, query).columns(['key "x"', "label"]).or_replace()
            .check_option("local").security_barrier().security_invoker(),
        "create_view_cascaded": p.create_view(view, query).check_option("cascaded"),
        "create_recursive_view": p.create_recursive_view(view, ['key "x"', "label"], query).or_replace().security_barrier(),
        "drop_view": p.drop_view(view, if_exists=True, behavior="cascade"),
        "create_materialized_view": p.create_materialized_view(view, query).columns(['key "x"']).if_not_exists().with_no_data(),
        "refresh_materialized_view": p.refresh_materialized_view(view),
        "refresh_concurrently": p.refresh_materialized_view(view, concurrently=True),
        "refresh_with_no_data": p.refresh_materialized_view(view, with_no_data=True),
        "drop_materialized_view": p.drop_materialized_view(view, if_exists=True),
        "create_table_as": p.create_table_as(view, query).columns(['key "x"', "label"]).if_not_exists().with_no_data(),
    }
