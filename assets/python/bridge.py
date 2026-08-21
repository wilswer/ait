#!/usr/bin/env python3
"""Internal bridge for AIT's typed Python tools.

This file is embedded into the Rust binary at compile time. It is executed via
`uv run --with pydantic` and communicates with Rust using JSON on stdout.
"""

from __future__ import annotations

import importlib.util
import inspect
import json
import sys
import traceback
from pathlib import Path
from typing import Any

from pydantic import ValidationError, create_model


def emit(value: dict[str, Any]) -> None:
    """Write exactly one machine-readable response to stdout."""
    print(json.dumps(value, default=str), flush=True)


def fail(
    kind: str,
    message: str,
    *,
    file: str | None = None,
    tool: str | None = None,
    parameter: str | None = None,
    hint: str | None = None,
    details: Any | None = None,
) -> None:
    error: dict[str, Any] = {"kind": kind, "message": message}
    if file is not None:
        error["file"] = file
    if tool is not None:
        error["tool"] = tool
    if parameter is not None:
        error["parameter"] = parameter
    if hint is not None:
        error["hint"] = hint
    if details is not None:
        error["details"] = details
    emit({"ok": False, "error": error})
    raise SystemExit(1)


def load_module(path_text: str):
    path = Path(path_text).expanduser().resolve()
    if not path.is_file():
        fail(
            "module_load_error",
            f"Python tool file does not exist: {path}",
            file=str(path),
        )

    module_name = f"ait_user_tools_{abs(hash(str(path)))}"
    try:
        spec = importlib.util.spec_from_file_location(module_name, path)
        if spec is None or spec.loader is None:
            fail(
                "module_load_error",
                f"Could not create an import specification for {path}.",
                file=str(path),
            )
        module = importlib.util.module_from_spec(spec)
        sys.modules[module_name] = module
        spec.loader.exec_module(module)
        return module, path
    except SyntaxError as exc:
        fail(
            "syntax_error",
            exc.msg,
            file=str(path),
            details={"line": exc.lineno, "offset": exc.offset, "text": exc.text},
        )
    except SystemExit:
        raise
    except Exception as exc:
        fail(
            "module_load_error",
            f"Failed to import {path.name}: {type(exc).__name__}: {exc}",
            file=str(path),
            details={"traceback": traceback.format_exc()},
        )


def parameter_fields(func, path: Path) -> dict[str, tuple[Any, Any]]:
    signature = inspect.signature(func)
    try:
        hints = inspect.get_annotations(func, eval_str=True)
    except Exception as exc:
        fail(
            "invalid_annotation",
            f"Could not resolve annotations: {type(exc).__name__}: {exc}",
            file=str(path),
            tool=func.__name__,
            details={"traceback": traceback.format_exc()},
        )

    if "return" not in hints:
        fail(
            "missing_return_annotation",
            "Tool functions must have a return type annotation.",
            file=str(path),
            tool=func.__name__,
            hint=f"Add `-> <type>` to `def {func.__name__}(...)`.",
        )

    fields: dict[str, tuple[Any, Any]] = {}
    for name, parameter in signature.parameters.items():
        if parameter.kind in (
            inspect.Parameter.POSITIONAL_ONLY,
            inspect.Parameter.VAR_POSITIONAL,
            inspect.Parameter.VAR_KEYWORD,
        ):
            fail(
                "unsupported_parameter_kind",
                f"Parameter '{name}' uses an unsupported parameter kind.",
                file=str(path),
                tool=func.__name__,
                parameter=name,
                hint="Use named parameters only; do not use positional-only parameters, *args, or **kwargs.",
            )

        if name not in hints:
            fail(
                "missing_annotation",
                f"Parameter '{name}' must have a type annotation.",
                file=str(path),
                tool=func.__name__,
                parameter=name,
                hint=f"Use, for example: `{name}: str`.",
            )

        default = (
            parameter.default
            if parameter.default is not inspect.Signature.empty
            else ...
        )
        fields[name] = (hints[name], default)

    return fields


def build_definition(func, path: Path) -> tuple[dict[str, Any], type[Any]]:
    docstring = inspect.getdoc(func)
    if not docstring:
        fail(
            "missing_docstring",
            "Tool functions must have a docstring for the LLM-facing description.",
            file=str(path),
            tool=func.__name__,
            hint=f'Add a docstring, e.g. `"""Describe what {func.__name__} does."""`.',
        )

    fields = parameter_fields(func, path)
    try:
        arguments_model = create_model(
            f"{func.__name__}_arguments",
            **fields,
        )
        schema = arguments_model.model_json_schema()
    except Exception as exc:
        fail(
            "schema_generation_error",
            f"Could not generate a JSON schema from the annotations: {type(exc).__name__}: {exc}",
            file=str(path),
            tool=func.__name__,
            hint="Use JSON-compatible Pydantic-supported annotations such as str, int, float, bool, Enum, list[T], dict[str, T], Optional[T], Literal[...], or BaseModel.",
            details={"traceback": traceback.format_exc()},
        )

    schema["additionalProperties"] = False
    return (
        {
            "name": func.__name__,
            "description": docstring,
            "input_schema": schema,
        },
        arguments_model,
    )


def discover(module: Any, path: Path) -> dict[str, tuple[Any, type[Any]]]:
    discovered: dict[str, tuple[Any, type[Any]]] = {}

    for name, value in sorted(vars(module).items()):
        if name.startswith("_"):
            continue
        if not inspect.isfunction(value):
            continue
        if value.__module__ != module.__name__:
            continue

        definition, arguments_model = build_definition(value, path)
        discovered[definition["name"]] = (value, arguments_model)

    return discovered


def command_discover(path_text: str) -> None:
    module, path = load_module(path_text)
    tools = discover(module, path)
    definitions = []
    for name in sorted(tools):
        func, _ = tools[name]
        definition, _ = build_definition(func, path)
        definitions.append(definition)
    emit({"ok": True, "tools": definitions})


def command_execute(path_text: str, tool_name: str, arguments_json: str) -> None:
    module, path = load_module(path_text)
    tools = discover(module, path)

    entry = tools.get(tool_name)
    if entry is None:
        fail(
            "unknown_tool",
            f"No public typed function named '{tool_name}' was found.",
            file=str(path),
            tool=tool_name,
        )

    try:
        raw_arguments = json.loads(arguments_json)
    except json.JSONDecodeError as exc:
        fail(
            "invalid_arguments_json",
            f"Tool arguments are not valid JSON: {exc.msg}",
            file=str(path),
            tool=tool_name,
        )

    if not isinstance(raw_arguments, dict):
        fail(
            "invalid_arguments_json",
            "Tool arguments must be a JSON object.",
            file=str(path),
            tool=tool_name,
        )

    func, arguments_model = entry
    try:
        validated = arguments_model.model_validate(raw_arguments)
    except ValidationError as exc:
        fail(
            "argument_validation_error",
            f"Invalid arguments for '{tool_name}'.",
            file=str(path),
            tool=tool_name,
            details=exc.errors(include_url=False),
        )

    try:
        result = func(**validated.model_dump(mode="python"))
        emit({"ok": True, "result": result})
    except Exception as exc:
        fail(
            "tool_execution_error",
            f"{type(exc).__name__}: {exc}",
            file=str(path),
            tool=tool_name,
            details={"traceback": traceback.format_exc()},
        )


def main() -> None:
    if len(sys.argv) < 3:
        fail(
            "bridge_usage_error",
            "Usage: bridge.py discover <tools.py> | execute <tools.py> <tool_name> <arguments_json>",
        )

    command = sys.argv[1]
    if command == "discover" and len(sys.argv) == 3:
        command_discover(sys.argv[2])
        return

    if command == "execute" and len(sys.argv) == 5:
        command_execute(sys.argv[2], sys.argv[3], sys.argv[4])
        return

    fail("bridge_usage_error", "Invalid bridge command or argument count.")


if __name__ == "__main__":
    main()
