import * as fs from "fs";
import * as path from "path";
import * as vscode from "vscode";
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
} from "vscode-languageclient/node";

let client: LanguageClient | undefined;

export async function activate(
  context: vscode.ExtensionContext,
): Promise<void> {
  context.subscriptions.push(
    vscode.commands.registerCommand("dataDict.restartServer", () =>
      restart(context),
    ),
  );
  await start(context);
}

export async function deactivate(): Promise<void> {
  await client?.stop();
  client = undefined;
}

async function start(context: vscode.ExtensionContext): Promise<void> {
  const config = vscode.workspace.getConfiguration("dataDict");
  const command = resolveServerPath(
    config.get<string>("server.path") ?? "data-dict",
    context.extensionPath,
  );
  const globs = config.get<string[]>("files") ?? ["**/data-dict.yaml"];

  // One executable acts as both CLI and language server; `lsp` selects the
  // server, speaking LSP over stdio.
  const serverOptions: ServerOptions = { command, args: ["lsp"] };

  const clientOptions: LanguageClientOptions = {
    documentSelector: globs.map((pattern) => ({ scheme: "file", pattern })),
  };

  client = new LanguageClient(
    "dataDict",
    "data-dict.yaml",
    serverOptions,
    clientOptions,
  );

  try {
    await client.start();
  } catch (err) {
    void vscode.window.showErrorMessage(
      `data-dict language server failed to start (\`${command} lsp\`). ` +
        "Build it with `cargo build -p data-dict-cli --features lsp`, or set " +
        `\`dataDict.server.path\`. ${String(err)}`,
    );
  }
}

async function restart(context: vscode.ExtensionContext): Promise<void> {
  await client?.stop();
  client = undefined;
  await start(context);
}

/// Resolve the server executable. An explicit, non-default setting wins (taken
/// relative to the workspace when not absolute). Otherwise look for a binary
/// built under `target/` — first in the opened workspace, then relative to the
/// extension itself (so running from source via F5 finds this repo's build no
/// matter which project is open) — before falling back to `data-dict` on `PATH`.
function resolveServerPath(configured: string, extensionPath: string): string {
  const folder = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;

  if (configured && configured !== "data-dict") {
    if (path.isAbsolute(configured)) {
      return configured;
    }
    return folder ? path.join(folder, configured) : configured;
  }

  // `editors/vscode/` is two levels below the workspace root, where `target/`
  // lives. Check the opened folder first, then the extension's own location.
  const roots = [folder, path.join(extensionPath, "..", "..")].filter(
    (root): root is string => Boolean(root),
  );
  const exe = process.platform === "win32" ? "data-dict.exe" : "data-dict";
  for (const root of roots) {
    for (const profile of ["release", "debug"]) {
      const candidate = path.join(root, "target", profile, exe);
      if (fs.existsSync(candidate)) {
        return candidate;
      }
    }
  }

  return "data-dict";
}
