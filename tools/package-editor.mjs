// Package the data-only grammar with the system ZIP tool. Nothing is published.
import { execFileSync } from "node:child_process";
import { constants, copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const extension = join(root, "editors/vscode/rastok-dsl");
const escapeXml = text => text.replaceAll("&", "&amp;").replaceAll('"', "&quot;").replaceAll("<", "&lt;").replaceAll(">", "&gt;").replaceAll("'", "&apos;");

function packageEditor(directory) {
  const manifest = JSON.parse(readFileSync(join(extension, "package.json"), "utf8"));
  if (!/^\d+\.\d+\.\d+$/.test(manifest.version)) throw new Error("the extension needs a three-part release version");
  const output = join(resolve(directory), `rastok-dsl-${manifest.version}.vsix`);
  if (existsSync(output)) throw new Error(`the package already exists: ${output}`);
  const staging = mkdtempSync(join(tmpdir(), "rastok-editor-staging-"));
  try {
    mkdirSync(join(staging, "extension/syntaxes"), { recursive: true });
    const files = ["package.json", "language-configuration.json", "README.md", "syntaxes/rkg.tmLanguage.json"];
    for (const file of files) copyFileSync(join(extension, file), join(staging, "extension", file));
    copyFileSync(join(root, "LICENSE"), join(staging, "extension/LICENSE"));
    writeFileSync(join(staging, "extension.vsixmanifest"), `<?xml version="1.0" encoding="utf-8"?>
<PackageManifest Version="2.0.0" xmlns="http://schemas.microsoft.com/developer/vsx-schema/2011" xmlns:d="http://schemas.microsoft.com/developer/vsx-schema-design/2011">
  <Metadata>
    <Identity Language="en-US" Id="${escapeXml(manifest.name)}" Version="${manifest.version}" Publisher="${escapeXml(manifest.publisher)}" />
    <DisplayName>${escapeXml(manifest.displayName)}</DisplayName>
    <Description xml:space="preserve">${escapeXml(manifest.description)}</Description>
    <License>extension/LICENSE</License>
    <Categories>Programming Languages</Categories>
    <Properties>
      <Property Id="Microsoft.VisualStudio.Code.Engine" Value="${escapeXml(manifest.engines.vscode)}" />
    </Properties>
  </Metadata>
  <Installation><InstallationTarget Id="Microsoft.VisualStudio.Code" /></Installation>
  <Dependencies />
  <Assets>
    <Asset Type="Microsoft.VisualStudio.Code.Manifest" Path="extension/package.json" Addressable="true" />
    <Asset Type="Microsoft.VisualStudio.Services.Content.Details" Path="extension/README.md" Addressable="true" />
    <Asset Type="Microsoft.VisualStudio.Services.Content.License" Path="extension/LICENSE" Addressable="true" />
  </Assets>
</PackageManifest>
`);
    writeFileSync(join(staging, "[Content_Types].xml"), `<?xml version="1.0" encoding="utf-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="json" ContentType="application/json" />
  <Default Extension="md" ContentType="text/markdown" />
  <Default Extension="vsixmanifest" ContentType="text/xml" />
  <Override PartName="/extension/LICENSE" ContentType="text/plain" />
</Types>
`);
    const archive = join(staging, "grammar.vsix");
    execFileSync("zip", ["-X", "-q", archive, "[Content_Types].xml", "extension.vsixmanifest", "extension/LICENSE", ...files.map(file => `extension/${file}`)], { cwd: staging });
    mkdirSync(resolve(directory), { recursive: true });
    copyFileSync(archive, output, constants.COPYFILE_EXCL);
    return output;
  } finally {
    rmSync(staging, { recursive: true });
  }
}

try {
  const [directory, ...rest] = process.argv.slice(2);
  if (!directory || rest.length) throw new Error("usage: node tools/package-editor.mjs <output-directory>");
  console.log(packageEditor(directory));
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
