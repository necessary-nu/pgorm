"""Explicit scaffold and wrapper-generation commands for application wheels,
and the declared models of a schema's DDL."""

import argparse
import json
from pathlib import Path

from . import CodegenError, emit, models_from_sql, scaffold
from .. import _native as p


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    create = commands.add_parser(
        "scaffold", help="write a new native application module project"
    )
    create.add_argument("config", type=Path)
    create.add_argument("--output", type=Path, required=True)
    create.add_argument("--pgorm-source", type=Path, required=True)
    generate = commands.add_parser(
        "emit", help="emit concrete wrappers from the installed application registry"
    )
    generate.add_argument("project", type=Path)
    models = commands.add_parser(
        "models", help="write the declared models of the tables a schema's DDL creates"
    )
    models.add_argument("schema", type=Path)
    models.add_argument("--output", type=Path, required=True)
    models.add_argument("--schema-name")
    options = parser.parse_args()
    try:
        if options.command == "scaffold":
            result = scaffold(
                json.loads(options.config.read_text()),
                options.output,
                pgorm_source=options.pgorm_source,
                base=options.config.resolve().parent,
            )
        elif options.command == "models":
            source = models_from_sql(options.schema.read_text(), schema_name=options.schema_name)
            options.output.write_text(source)
            result = options.output
        else:
            result = emit(options.project)
    except (CodegenError, p.ConstructionError, OSError, json.JSONDecodeError) as error:
        parser.error(str(error))
    print(result)


if __name__ == "__main__":
    main()
