"""Import the audited upstream .strings dictionaries into Rust JSON resources."""
import json
import pathlib
import re
import subprocess
import sys

target = pathlib.Path(__file__).resolve().parents[1] / "quotascope-core/src/translations"
target.mkdir(exist_ok=True)
ref = sys.argv[1] if len(sys.argv) > 1 else "b396306c9b19c5c24764a97b9308d86d7959abbc"
for language in ("zh-Hant", "ja", "ko"):
    source = subprocess.check_output(
        ["git", "show", f"{ref}:Sources/Pulse/Resources/{language}.lproj/Localizable.strings"]
    ).decode("utf-8")
    pairs = re.findall(r'("(?:\\.|[^"\\])*")\s*=\s*("(?:\\.|[^"\\])*")\s*;', source)
    translations = {json.loads(key).replace("Pulse", "QuotaScope"): json.loads(value).replace("Pulse", "QuotaScope") for key, value in pairs}
    (target / f"{language}.json").write_text(json.dumps(translations, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(language, len(translations))
