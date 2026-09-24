"""Fixtures and immutable, independently evaluated success criteria."""
from copy import deepcopy
from pathlib import Path
from .tools import safe_path, MAX_FILE_BYTES, MAX_WORKSPACE_BYTES, MAX_FILES

REVENUE = {
    "id": "revenue-audit", "title": "Audit a sales report", "tag": "Data analysis",
    "description": "Read an order ledger, calculate revenue, and produce a checked JSON report.",
    "prompt": "Read orders.csv. For paid orders only, calculate total revenue as quantity * unit_price. "
              "Write report.json with exactly these keys: paid_orders (count), revenue (integer), "
              "top_product (product with the highest total paid revenue). Use the calculate tool to "
              "check arithmetic. Do not modify orders.csv. Finish with one concise sentence.",
    "files": {"orders.csv": "order_id,product,quantity,unit_price,status\n"
              "101,Keyboard,2,80,paid\n102,Mouse,3,25,paid\n103,Keyboard,1,80,refunded\n"
              "104,Monitor,1,240,paid\n105,Mouse,2,25,paid\n106,Monitor,2,240,pending\n"},
    "checks": [{"name": "Report created", "type": "exists", "path": "report.json"},
               {"name": "Revenue and product are correct", "type": "json_equals", "path": "report.json",
                "expected": {"paid_orders": 4, "revenue": 525, "top_product": "Monitor"}}],
    "demo": "revenue",
}
REVENUE["checks"].append({"name": "Source ledger preserved", "type": "text_equals", "path": "orders.csv",
                          "expected": REVENUE["files"]["orders.csv"]})
CONFIG = {
    "id": "config-repair", "title": "Repair a service config", "tag": "File editing",
    "description": "Fix configuration values while preserving the service name and region.",
    "prompt": "Read service.json. Set retries to 3, timeout_seconds to 30, and debug to false. "
              "Preserve the other values. Write the corrected service.json. Finish with a brief summary.",
    "files": {"service.json": '{\n  "service": "relay-worker",\n  "region": "ap-southeast",\n'
              '  "retries": 0,\n  "timeout_seconds": 2,\n  "debug": true\n}\n'},
    "checks": [{"name": "Config values and preserved fields", "type": "json_equals", "path": "service.json",
                "expected": {"service": "relay-worker", "region": "ap-southeast", "retries": 3,
                             "timeout_seconds": 30, "debug": False}}], "demo": "config",
}
FAILURE = deepcopy(REVENUE)
FAILURE.update(id="failure-lab", title="Catch a wrong answer", tag="Verification lab",
               description="The scripted demo makes an arithmetic mistake. Watch independent checks catch it.",
               demo="wrong")
TASKS = {t["id"]: t for t in (REVENUE, CONFIG, FAILURE)}


def validate_task(task):
    if not isinstance(task, dict):
        raise ValueError("Task must be a JSON object.")
    for key in ("title", "prompt"):
        if not isinstance(task.get(key), str) or not task[key].strip() or len(task[key]) > 12_000:
            raise ValueError(f"Task {key} must be nonempty text (up to 12,000 characters).")
    files, checks = task.get("files", {}), task.get("checks", [])
    if not isinstance(files, dict) or len(files) > MAX_FILES:
        raise ValueError("Files must be an object with at most 100 entries.")
    root = Path("/relay-validation-root")
    normalized = set()
    for path, content in files.items():
        canonical = str(safe_path(root, path))
        if canonical in normalized:
            raise ValueError("Duplicate normalized file path.")
        normalized.add(canonical)
        if not isinstance(content, str) or len(content.encode()) > MAX_FILE_BYTES:
            raise ValueError("Each file must be UTF-8 text up to 64 KB.")
    if sum(len(v.encode()) for v in files.values()) > MAX_WORKSPACE_BYTES:
        raise ValueError("Initial workspace exceeds 1 MB.")
    if not isinstance(checks, list) or len(checks) > 30:
        raise ValueError("Checks must be a list with at most 30 entries.")
    for check in checks:
        if not isinstance(check, dict) or check.get("type") not in {"exists", "contains", "text_equals", "json_equals"}:
            raise ValueError("Check type must be exists, contains, text_equals, or json_equals.")
        safe_path(root, check.get("path"))
        if check["type"] != "exists" and "expected" not in check:
            raise ValueError("This check needs an expected value.")
        if check["type"] in {"contains", "text_equals"} and not isinstance(check["expected"], str):
            raise ValueError("A text check requires text.")
    return {**deepcopy(task), "files": files, "checks": checks}
