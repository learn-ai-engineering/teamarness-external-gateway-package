// Generated from ui-contract.json by teamarness-contract-types. Do not edit.

export type CommandChange = {
  readonly "disposition": "install" | "unchanged" | "refresh" | "explicit-repair";
  readonly "installed": CommandVersion | null;
  readonly "proposed": CommandVersion;
};

export type CommandVersion = {
  readonly "installation": string;
  readonly "sha256": string;
  readonly "version": string;
};

export type Contract = {
  readonly "available": boolean;
  readonly "contract": string;
  readonly "displayName": string | null;
  readonly "guideEligibility": "eligible" | "missing" | "unavailable";
  readonly "hostEnabled": boolean;
  readonly "major": number;
  readonly "provider": string | null;
  readonly "selected": boolean;
};

export type ObservedContract = {
  readonly "contract": string;
  readonly "major": number;
};

export type SetupInspection = {
  readonly "cli": {
    readonly "change": "none" | "install" | "refresh" | "explicit-repair";
    readonly "issue": ("external_gateway.output_unsafe" | "external_gateway.output_collision" | "external_gateway.output_state_invalid" | "external_gateway.installer_busy" | "external_gateway.setup_preview_stale" | "external_gateway.output_consent_required" | "external_gateway.reload_required" | "external_gateway.output_repair_required" | "external_gateway.output_storage_failed" | "external_gateway.unavailable" | "external_gateway.client_store_invalid" | "external_gateway.connection_origin_conflict" | "external_gateway.credential_consent_required" | "external_gateway.publication_required" | "external_gateway.setup_repair_required" | "external_gateway.setup_state_invalid" | "external_gateway.storage_failed") | null;
    readonly "path": string;
    readonly "state": "missing" | "installed" | "blocked" | "unavailable";
    readonly "version": string | null;
  };
  readonly "connection": {
    readonly "authority": "none" | "current" | "revoked" | "disconnected" | "unavailable";
    readonly "issue": ("external_gateway.output_unsafe" | "external_gateway.output_collision" | "external_gateway.output_state_invalid" | "external_gateway.installer_busy" | "external_gateway.setup_preview_stale" | "external_gateway.output_consent_required" | "external_gateway.reload_required" | "external_gateway.output_repair_required" | "external_gateway.output_storage_failed" | "external_gateway.unavailable" | "external_gateway.client_store_invalid" | "external_gateway.connection_origin_conflict" | "external_gateway.credential_consent_required" | "external_gateway.publication_required" | "external_gateway.setup_repair_required" | "external_gateway.setup_state_invalid" | "external_gateway.storage_failed") | null;
    readonly "repair": "none" | "publication" | "replace-credentials" | "resume" | "cleanup";
    readonly "state": "not-connected" | "connected" | "needs-repair" | "conflict" | "unavailable";
  };
  readonly "recovery": {
    readonly "intent": ("connect" | "repair" | "rotate" | "disconnect" | "install-skill") | null;
    readonly "state": "none" | "pending" | "invalid";
  };
};

export type SetupStatus = {
  readonly "access": "unchanged" | "pending" | "configured" | "revoked" | "repair-required";
  readonly "command": "unchanged" | "pending" | "installed";
  readonly "commandPath": string | null;
  readonly "error": string | null;
  readonly "intent": ("connect" | "repair" | "rotate" | "disconnect" | "install-skill" | "install-cli") | null;
  readonly "operationId": string | null;
  readonly "phase": "idle" | "accepted" | "prepared" | "outputs" | "committed" | "complete" | "repair-required" | "conflict" | "cleanup-pending";
  readonly "skill": "not-requested" | "pending" | "installed";
  readonly "skillPath": string | null;
};

export type Status = {
  readonly "desiredEnabled": boolean;
  readonly "port": number | null;
  readonly "running": boolean;
  readonly "selectedContracts": number;
  readonly "selectedTools": number;
  readonly "storageFailed": boolean;
  readonly "tokens": readonly Token[];
  readonly "warning": string | null;
};

export type Token = {
  readonly "generation": number;
  readonly "id": string;
  readonly "label": string;
  readonly "managed"?: boolean;
};

export type Tool = {
  readonly "delivery": "response" | "stream";
  readonly "eligible": boolean;
  readonly "fingerprint": string;
  readonly "kind": "query" | "command" | "event";
  readonly "port": string;
  readonly "reviewRequired": boolean;
  readonly "selected": boolean;
};

export type PackageContract = {
  readonly requests: {
    readonly "external-gateway.catalog.page": { readonly request: {
        readonly "contract"?: string;
        readonly "cursor"?: string;
        readonly "offset"?: number;
      }; readonly response: {
        readonly "contracts": readonly Contract[];
        readonly "nextCursor": string | null;
        readonly "nextOffset": number | null;
        readonly "tools": readonly Tool[];
      } };
    readonly "external-gateway.disable-and-revoke-all": { readonly request: Record<string, never>; readonly response: {
        readonly "ok": boolean;
        readonly "warning": string | null;
      } };
    readonly "external-gateway.listener.set": { readonly request: {
        readonly "enabled": boolean;
      }; readonly response: {
        readonly "ok": boolean;
        readonly "warning": string | null;
      } };
    readonly "external-gateway.package.set": { readonly request: {
        readonly "contracts": readonly ObservedContract[];
        readonly "enabled": boolean;
        readonly "provider": string | null;
      }; readonly response: {
        readonly "ok": boolean;
        readonly "warning": string | null;
      } };
    readonly "external-gateway.port.set": { readonly request: {
        readonly "port": number;
      }; readonly response: {
        readonly "ok": boolean;
        readonly "warning": string | null;
      } };
    readonly "external-gateway.setup.apply": { readonly request: {
        readonly "planId": string;
      }; readonly response: {
        readonly "operationId": string;
      } };
    readonly "external-gateway.setup.inspect": { readonly request: Record<string, never>; readonly response: SetupInspection };
    readonly "external-gateway.setup.preview": { readonly request: {
        readonly "intent": "connect" | "repair" | "rotate" | "disconnect" | "install-skill" | "install-cli";
        readonly "selectionId"?: string;
      }; readonly response: {
        readonly "commandChange": CommandChange | null;
        readonly "commandPath": string;
        readonly "credentialChange": boolean;
        readonly "intent": "connect" | "repair" | "rotate" | "disconnect" | "install-skill" | "install-cli";
        readonly "planId": string;
        readonly "skillChange": "not-requested" | "publish" | "unchanged";
        readonly "skillPath": string | null;
      } };
    readonly "external-gateway.setup.status": { readonly request: Record<string, never>; readonly response: SetupStatus };
    readonly "external-gateway.status.get": { readonly request: Record<string, never>; readonly response: Status };
    readonly "external-gateway.token.create": { readonly request: {
        readonly "label": string;
      }; readonly response: {
        readonly "bearer": string;
        readonly "id": string;
      } };
    readonly "external-gateway.token.regenerate": { readonly request: {
        readonly "id": string;
      }; readonly response: {
        readonly "bearer": string;
        readonly "id": string;
      } };
    readonly "external-gateway.token.revoke": { readonly request: {
        readonly "id": string;
      }; readonly response: {
        readonly "ok": boolean;
        readonly "warning": string | null;
      } };
    readonly "external-gateway.tool.set": { readonly request: {
        readonly "contract": string;
        readonly "enabled": boolean;
        readonly "fingerprint": string;
        readonly "major": number;
        readonly "port": string;
        readonly "provider": string;
      }; readonly response: {
        readonly "ok": boolean;
        readonly "warning": string | null;
      } };
  };
  readonly subscriptions: {
    readonly "external-gateway.setup.choose-skill-folder": { readonly request: Record<string, never>; readonly event: {
        readonly "outcome": "selected";
        readonly "path": string;
        readonly "selectionId": string;
      } | {
        readonly "outcome": "cancelled";
      } };
    readonly "external-gateway.setup.watch": { readonly request: Record<string, never>; readonly event: SetupStatus };
    readonly "external-gateway.status.watch": { readonly request: Record<string, never>; readonly event: Status };
  };
};
