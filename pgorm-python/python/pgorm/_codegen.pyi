def _models_from_sql(
    ddl: str,
    schema_name: str | None,
    unkinded_disjoint: list[tuple[tuple[str | None, str], list[tuple[str | None, str]], str | None]],
) -> str: ...
