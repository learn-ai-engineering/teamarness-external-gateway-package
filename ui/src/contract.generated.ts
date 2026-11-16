// Generated from ui-contract.json by teamarness-contract-types. Do not edit.

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
    readonly "external-gateway.status.watch": { readonly request: Record<string, never>; readonly event: Status };
  };
};
