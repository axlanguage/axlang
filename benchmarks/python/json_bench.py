import json
import sys


PAYLOAD = '{ "task" : "summarize", "ok" : true, "tokens" : 2048, "agent" : { "name" : "codex", "limits" : { "tokens" : 2048 } }, "steps" : [{ "name" : "read" }, { "name" : "verify" }], "tools" : ["read", "write", "verify"] }'


def kind(value):
    if value is None:
        return "null"
    if isinstance(value, bool):
        return "bool"
    if isinstance(value, (int, float)):
        return "number"
    if isinstance(value, str):
        return "string"
    if isinstance(value, list):
        return "array"
    if isinstance(value, dict):
        return "object"
    return "missing"


def json_score(n):
    acc = 0
    for _ in range(n):
        data = json.loads(PAYLOAD)
        if "task" in data and data.get("ok") is True:
            task = data["task"]
            agent = data["agent"]["name"]
            first_step = data["steps"][0]["name"]
            task_kind = kind(task)
            nested_kind = kind(data["agent"]["limits"]["tokens"])
            tool_count = len(data["tools"])
            first_tool = data["tools"][0]
            tokens = data["agent"]["limits"]["tokens"]
            if tokens == 2048 and "verify" in data["tools"] and "steps" in data.keys():
                acc += len(task)
                acc += len(agent)
                acc += len(first_step)
                acc += len(task_kind)
                acc += len(nested_kind)
                acc += len(first_tool)
                acc += tool_count
                acc += tokens % 97
    return acc % 251


sys.exit(json_score(10000))
