"""Collect original dependency license files for a Windows binary release."""
import json
import shutil
import sys
from pathlib import Path

repo = Path(__file__).resolve().parent.parent
metadata = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8-sig"))
dest = Path(sys.argv[2]).resolve()
dest.mkdir(parents=True, exist_ok=True)
index = ["# Binary dependency licenses", "", "License expressions below are upstream declarations. Original notices follow in each package directory.", ""]


def collect(root, target):
    count = 0
    for source in root.rglob("*"):
        if source.is_file() and source.name.lower().startswith(("license", "copying", "notice", "copyright", "thirdpartynotices", "third_party_licenses")):
            out = target / source.relative_to(root)
            out.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, out)
            count += 1
    return count


missing = []
resolved = {node["id"] for node in metadata["resolve"]["nodes"]}
for package in metadata["packages"]:
    if package["id"] in metadata["workspace_members"] or package["id"] not in resolved:
        continue
    name = f'{package["name"]}-{package["version"]}'
    index.append(f'- {name}: {package.get("license")}; {package.get("repository") or "https://crates.io/crates/" + package["name"]}')
    root = Path(package["manifest_path"]).parent
    count = collect(root, dest / "cargo" / name)
    supplement = repo / "docs/licenses/dependencies" / package["name"]
    if supplement.exists():
        count += collect(supplement, dest / "cargo" / name)
    if package.get("license") == "MPL-2.0" or package["name"] == "esaxx-rs":
        # Supply unmodified MPL source and the patched esaxx sources, including inline notices.
        shutil.copytree(root, dest / "sources" / name, dirs_exist_ok=True,
                        ignore=shutil.ignore_patterns("target", ".git", ".cargo-ok"))
    if not count:
        missing.append(name)

for assets in (repo / "apps").glob("*/obj/project.assets.json"):
    data = json.loads(assets.read_text(encoding="utf-8-sig"))
    for name, package in data["libraries"].items():
        if package["type"] != "package":
            continue
        for folder in data["packageFolders"]:
            root = Path(folder) / package["path"]
            if root.exists():
                collect(root, dest / "nuget" / name.replace("/", "-"))
                break

for source in (repo / "docs/licenses").glob("*.txt"):
    shutil.copyfile(source, dest / source.name)
shutil.copyfile(repo / "docs/MOZC-DATE-LICENSE.txt", dest / "MOZC-DATE-LICENSE.txt")
(dest / "DEPENDENCIES.md").write_text("\n".join(index) + "\n", encoding="utf-8")
if missing:
    raise SystemExit("Missing original license files: " + ", ".join(missing))
print(f"Collected licenses for {len(index) - 4} Cargo dependencies and restored NuGet packages")
