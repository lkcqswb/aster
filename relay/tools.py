"""Explicitly registered tools confined to one run's workspace."""
import ast
import json
import math
import operator
from pathlib import Path

MAX_FILE_BYTES = 64_000
MAX_WORKSPACE_BYTES = 1_000_000
MAX_FILES = 100


def safe_path(root: Path, name: str) -> Path:
    if not isinstance(name, str) or not name or len(name) > 240:
        raise ValueError("Use a nonempty relative path, up to 240 characters.")
    rel = Path(name)
    if rel.is_absolute() or ".." in rel.parts or "\x00" in name:
        raise ValueError("Path must stay inside this run's workspace.")
    root = root.resolve()
    path = (root / rel).resolve()
    if path == root or root not in path.parents:
        raise ValueError("Path must name a file inside this run's workspace.")
    return path


def snapshot(root: Path) -> dict:
    result = {}
    total = 0
    for path in sorted(root.rglob("*")):
        if path.is_symlink():
            raise ValueError("Symbolic links are not supported.")
        if path.is_file():
            if path.stat().st_size > MAX_FILE_BYTES:
                raise ValueError("File exceeds 64 KB.")
            content = path.read_text(encoding="utf-8")
            total += len(content.encode())
            result[str(path.relative_to(root))] = content
            if total > MAX_WORKSPACE_BYTES or len(result) > MAX_FILES:
                raise ValueError("Workspace limit exceeded.")
    return result


def write_file(root: Path, path: str, content: str) -> dict:
    target = safe_path(root, path)
    if not isinstance(content, str) or len(content.encode()) > MAX_FILE_BYTES:
        raise ValueError("Content must be text no larger than 64 KB.")
    files = snapshot(root)
    files[path] = content
    if len(files) > MAX_FILES or sum(len(v.encode()) for v in files.values()) > MAX_WORKSPACE_BYTES:
        raise ValueError("Workspace limit exceeded.")
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(content, encoding="utf-8")
    return {"path": path, "bytes": len(content.encode())}


def calculate(expression: str) -> dict:
    if not isinstance(expression, str) or len(expression) > 500:
        raise ValueError("Expression must be text up to 500 characters.")
    tree = ast.parse(expression, mode="eval")
    if len(list(ast.walk(tree))) > 100:
        raise ValueError("Expression is too complex.")
    binary = {ast.Add: operator.add, ast.Sub: operator.sub, ast.Mult: operator.mul,
              ast.Div: operator.truediv, ast.FloorDiv: operator.floordiv, ast.Mod: operator.mod}

    def visit(node):
        if isinstance(node, ast.Constant) and type(node.value) in (int, float):
            value = node.value
        elif isinstance(node, ast.BinOp) and type(node.op) in binary:
            value = binary[type(node.op)](visit(node.left), visit(node.right))
        elif isinstance(node, ast.UnaryOp) and isinstance(node.op, (ast.UAdd, ast.USub)):
            value = visit(node.operand) * (-1 if isinstance(node.op, ast.USub) else 1)
        else:
            raise ValueError("Only numbers, parentheses, and + - * / // % are supported.")
        if abs(value) > 1e12 or not math.isfinite(value):
            raise ValueError("Calculation exceeds the numeric limit.")
        return value

    return {"expression": expression, "result": visit(tree.body)}


def schema(name, description, properties):
    return {"name": name, "description": description, "input_schema": {
        "type": "object", "properties": {k: {"type": "string", "description": v}
                                                for k, v in properties.items()},
        "required": list(properties), "additionalProperties": False}}


TOOL_SCHEMAS = [
    schema("list_files", "List all text files in the isolated task workspace.", {}),
    schema("read_file", "Read a UTF-8 file from the task workspace.", {"path": "Relative file path"}),
    schema("write_file", "Create or replace a UTF-8 file in the task workspace.",
           {"path": "Relative file path", "content": "Complete new file content"}),
    schema("calculate", "Evaluate arithmetic without running code.", {"expression": "Arithmetic expression"}),
]


def execute(root: Path, name: str, arguments: dict) -> dict:
    specs = {s["name"]: s for s in TOOL_SCHEMAS}
    if name not in specs:
        raise ValueError(f"Unknown tool: {name}")
    expected = set(specs[name]["input_schema"]["properties"])
    if not isinstance(arguments, dict) or set(arguments) != expected:
        raise ValueError(f"Expected arguments: {', '.join(sorted(expected)) or 'none'}")
    if any(not isinstance(v, str) for v in arguments.values()):
        raise ValueError("Tool arguments must be strings.")
    if name == "list_files":
        return {"files": [{"path": k, "bytes": len(v.encode())} for k, v in snapshot(root).items()]}
    if name == "read_file":
        path = safe_path(root, arguments["path"])
        if not path.is_file():
            raise ValueError("File not found.")
        if path.stat().st_size > MAX_FILE_BYTES:
            raise ValueError("File exceeds 64 KB.")
        return {"path": arguments["path"], "content": path.read_text(encoding="utf-8")}
    if name == "write_file":
        return write_file(root, **arguments)
    return calculate(**arguments)


def verify(root: Path, checks: list) -> list:
    results = []
    for check in checks:
        result = {"name": check.get("name", check["type"]), "path": check["path"], "passed": False}
        try:
            path = safe_path(root, check["path"])
            if not path.is_file():
                raise ValueError("Required file is missing.")
            if path.stat().st_size > MAX_FILE_BYTES:
                raise ValueError("File exceeds 64 KB.")
            content = path.read_text(encoding="utf-8")
            if check["type"] == "exists":
                result.update(passed=True, detail="File exists.")
            elif check["type"] == "contains":
                result.update(passed=check["expected"] in content, detail="Required text comparison.")
            elif check["type"] == "text_equals":
                result.update(passed=check["expected"] == content, detail="Exact text comparison.")
            elif check["type"] == "json_equals":
                actual = json.loads(content)
                # JSON booleans must not silently compare equal to numbers.
                canonical = lambda v: json.dumps(v, sort_keys=True, ensure_ascii=False, allow_nan=False)
                result.update(passed=canonical(actual) == canonical(check["expected"]),
                              detail="Exact JSON comparison (key order ignored).", actual=actual,
                              expected=check["expected"])
            else:
                raise ValueError("Unknown check type.")
        except (ValueError, OSError, TypeError) as exc:
            result["detail"] = str(exc)
        results.append(result)
    return results
