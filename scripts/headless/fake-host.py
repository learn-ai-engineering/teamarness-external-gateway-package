"""Test double for the headless Host operator CLI, for external-gateway-setup tests only.

State lives in the JSON file named by FAKE_HOST_STATE. Every mutation is appended to
state["log"]. state["faults"] injects failures, keyed by operation name:
  "uncertain"  -> exit 1 with outcomeUncertain true (after recording the attempt)
  "fail"       -> exit 1 with a certain read failure
"""
import json
import os
import sys

STATE = os.environ["FAKE_HOST_STATE"]
IDENTITY = "dev.teamarness.external-gateway"
DIGEST = "d" * 64


def load():
    with open(STATE) as f:
        return json.load(f)


def save(s):
    with open(STATE, "w") as f:
        json.dump(s, f)


def out(value, code=0):
    print(json.dumps(value))
    sys.exit(code)


def error(code, uncertain, details=None):
    print(f"{code} fake error", file=sys.stderr)
    out({"code": code, "outcomeUncertain": uncertain, "retryable": False, "details": details or {}}, 1)


def flag(args, name):
    return args[args.index(name) + 1]


def fault(s, op, mutation):
    kind = s.get("faults", {}).get(op)
    if kind == "uncertain":
        save(s)
        error("package_error", True, {"code": "external_gateway.storage_failed"})
    if kind == "fail":
        error("package_error", mutation, {"code": "external_gateway.busy"})


def main(argv):
    assert argv[0] == "--data-dir", argv
    s = load()
    args = argv[2:]
    if args[:2] == ["packages", "list"]:
        regs = [{"directory": s["registered"], "expectedIdentity": IDENTITY, "override": False,
                 "status": {"kind": "admitted"}, "tier": "personal"}] if s.get("registered") else []
        out({"registrations": regs, "packages": [], "instances": [], "diagnostics": [], "revision": 1, "state": "running"})
    if args[:2] == ["packages", "inspect"]:
        out({"identity": IDENTITY, "version": "1.1.0"})
    if args[:2] == ["packages", "install"]:
        body = json.load(sys.stdin)
        assert body == {"acceptedRisks": ["filesystem", "network", "process"]}, body
        s["log"].append("packages.install")
        fault(s, "packages.install", True)
        s["registered"] = flag(args, "--directory")
        save(s)
        out({"kind": "personal_installed", "revision": 2})
    if args[:2] == ["package", "operations"]:
        if not s.get("registered"):
            error("not_found", False)
        out({"contractDigest": DIGEST, "operations": []})
    if args[:2] == ["package", "request"]:
        assert flag(args, "--contract-digest") == DIGEST
        assert flag(args, "--workspace") == s["workspace"]
        op = flag(args, "--operation").removeprefix("external-gateway.")
        body = json.load(sys.stdin)
        return request(s, op, body)
    error("invalid_input", False)


MUTATIONS = {"setup.apply", "listener.set", "package.set", "tool.set"}


def request(s, op, body):
    mutation = op in MUTATIONS
    if mutation:
        s["log"].append(op + (" " + json.dumps(body, sort_keys=True) if op == "tool.set" else ""))
    fault(s, op, mutation)
    g = s["gateway"]
    if op == "setup.inspect":
        cli_state = "installed" if g["cli"] else "missing"
        out({"cli": {"state": cli_state, "change": "none" if g["cli"] else "install", "issue": g.get("cli_issue"),
                     "path": s["tmx"], "version": "1.1.0" if g["cli"] else None},
             "connection": {"state": g["connection"], "authority": "current" if g["connection"] == "connected" else "none",
                            "issue": None, "repair": g.get("repair", "none")},
             "recovery": {"state": "none", "intent": None}})
    if op == "setup.preview":
        g["plan"] = body["intent"]
        save(s)
        out({"planId": "plan-secret-0001", "intent": body["intent"]})
    if op == "setup.apply":
        assert body == {"planId": "plan-secret-0001"}
        intent = g.pop("plan")
        if intent == "install-cli":
            g["cli"] = True
        elif intent == "connect":
            g["connection"] = "connected"
        g["op"] = {"intent": intent, "polls": g.get("slow_polls", 0)}
        save(s)
        out({"operationId": "op-1"})
    if op == "setup.status":
        o = g.get("op") or {"intent": None, "polls": 0}
        if o["polls"] > 0:
            o["polls"] -= 1
            save(s)
            out({"operationId": "op-1", "phase": "outputs", "error": None})
        result = {"operationId": "op-1", "phase": "complete", "error": None, "intent": o["intent"],
                  "command": "installed" if o["intent"] == "install-cli" else "unchanged",
                  "access": "configured" if o["intent"] == "connect" else "unchanged"}
        out(result)
    if op == "status.get":
        out({"desiredEnabled": g["listener"], "running": g["listener"], "port": 40000 if g["listener"] else None,
             "warning": None, "selectedContracts": 0, "selectedTools": 0, "storageFailed": False, "tokens": []})
    if op == "listener.set":
        g["listener"] = body["enabled"]
        save(s)
        out({"ok": True, "warning": None})
    if op == "catalog.page":
        if "contract" in body:
            c = s["catalog"][body["contract"]]
            offset = body.get("offset", 0)
            tools = []
            for name in c["ports"][offset:offset + 32]:
                selected = name in c["selected_tools"]
                tools.append({"port": name, "fingerprint": "fp-" + name, "eligible": name != "runtime.guide",
                              "selected": selected, "reviewRequired": name in c.get("review", []),
                              "kind": "query", "delivery": "ordinary"})
            out({"contracts": [], "nextCursor": None, "tools": tools,
                 "nextOffset": offset + 32 if offset + 32 < len(c["ports"]) else None})
        names = sorted(s["catalog"])
        start = int(body.get("cursor", "0"))
        page = names[start:start + 2]
        rows = [{"contract": n, "major": 1, "provider": s["catalog"][n]["provider"], "displayName": n,
                 "available": True, "hostEnabled": s["catalog"][n]["enabled"], "selected": s["catalog"][n]["enabled"],
                 "guideEligibility": "eligible"} for n in page]
        out({"contracts": rows, "nextCursor": str(start + 2) if start + 2 < len(names) else None,
             "tools": [], "nextOffset": None})
    if op == "package.set":
        c = s["catalog"][body["contracts"][0]["contract"]]
        assert body["provider"] == c["provider"] and body["enabled"] is True
        c["enabled"] = True
        save(s)
        out({"ok": True, "warning": None})
    if op == "tool.set":
        c = s["catalog"][body["contract"]]
        assert c["enabled"] and body["fingerprint"] == "fp-" + body["port"]
        c["selected_tools"].append(body["port"])
        save(s)
        out({"ok": True, "warning": None})
    error("invalid_input", False)


main(sys.argv[1:])
