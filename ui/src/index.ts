import {
  adoptPackageStyles,
  describePackageError,
  isAbortError,
  typedPackageClient,
  type PackageMountContext,
  type PackageUnmount,
} from "@teamarness/package-ui-sdk";

import type { PackageContract } from "./contract.generated";
import { PACKAGE_STYLES } from "./styles";

/**
 * Mounts the Package UI.
 *
 * It returns its unmount synchronously, before the first request answers, so the Shell can take
 * the mount down while a request is still in flight.
 */
export function mount(context: PackageMountContext): PackageUnmount {
  const client = typedPackageClient<PackageContract>(context.client);
  const cleanupStyles = adoptPackageStyles(context.root, PACKAGE_STYLES);

  const article = document.createElement("article");
  const heading = document.createElement("h1");
  heading.textContent = context.package.displayName;
  const message = document.createElement("p");
  message.textContent = "Loading…";
  const reads = document.createElement("p");
  reads.className = "reads";
  article.append(heading, message, reads);
  context.root.append(article);

  const pending = new AbortController();
  let unmounted = false;

  const fail = (error: unknown): void => {
    if (unmounted || isAbortError(error)) return;
    message.textContent = "";
    const alert = document.createElement("p");
    alert.className = "error";
    alert.setAttribute("role", "alert");
    alert.textContent = describePackageError(error);
    article.append(alert);
  };

  void client
    .request("external-gateway.status.get", {}, { signal: pending.signal })
    .then((response) => {
      if (unmounted) return;
      message.textContent = response.desiredEnabled ? "Read-only access configured" : "Read-only access disabled";
      reads.textContent = `${response.tokens.length} tokens. HTTP listener is not available in this build.`;
    })
    .catch(fail);

  return () => {
    if (unmounted) return;
    unmounted = true;
    pending.abort();
    article.remove();
    cleanupStyles();
  };
}
