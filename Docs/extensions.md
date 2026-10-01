# Windows extensions

An extension is an executable in a folder that reports one account's usage.
QuotaScope reads its manifest while scanning and runs it only after the account
is enabled in Settings → Accounts → Extensions. The program owns its login;
QuotaScope does not pass stored provider credentials to it.

## Install

Place the folder at `%APPDATA%\QuotaScope\Extensions\<folder>\`. Add
`quotascope-extension.json` alongside a Windows executable:

```json
{
  "schemaVersion": 1,
  "id": "example-quota",
  "name": "Example quota",
  "executable": "reader.exe",
  "timeoutSeconds": 20
}
```

The id is 1–64 lowercase ASCII letters, digits, dots, dashes or underscores,
starting with a letter or digit, and unique among installed extensions. The
name is nonempty and truncated to 60 characters. The executable must resolve
inside its own folder. This is a direct process launch, not a shell command;
compile a reader executable rather than putting shell arguments in this field.
The account id is `extension#example-quota`.

The working directory is the extension folder. Timeout is clamped to 1–60
seconds, default 20. QuotaScope reads stdout with a 256 KiB limit, requires a
successful exit, discards stderr and terminates a timed-out process. The
extension should emit exactly one JSON object and no diagnostic text on stdout.

## Report

```json
{
  "schemaVersion": 1,
  "status": "ok",
  "plan": "Example plan",
  "limits": [
    {
      "id": "daily",
      "label": "Daily allowance",
      "used": 25,
      "limit": 100,
      "windowSeconds": 86400,
      "resetsAt": "2026-10-02T00:00:00+08:00"
    }
  ],
  "balance": { "amount": 12.5, "currency": "USD" }
}
```

`status` defaults to `ok`. Other accepted statuses are `signedOut`,
`unreachable`, `rateLimited`, `serverError`, `noLimits`. A limit needs a nonempty
label and either `usedPercent` or `used` together with a positive `limit`.
Unknown percentages are omitted. A stated percentage at 100% is exhausted.
`windowSeconds` must be positive to support a clock; it is not inferred from a
label. `resetsAt` is an ISO-8601 timestamp. Duplicate limit ids are omitted.
Balance requires a finite amount and a three-letter uppercase currency code;
negative balances are permitted. A balance alone is valid and can receive the
generic balance ring. A report with neither usable limits nor balance says
that no limits were reported.

## Environment and boundary

`QUOTASCOPE_EXTENSION_ID` and `QUOTASCOPE_EXTENSION_SCHEMA=1` identify the run.
The process inherits only the allowlisted home/app-data, temporary-directory,
system, locale, PATH and proxy variables. Arbitrary environment API tokens are
not passed. PATH follows the Windows user session; macOS paths and `PULSE_*`
environment variable names are not used.

The program is out of process, not sandboxed. It can use the filesystem,
network and credentials available to its Windows user. Install readers you
trust. Disabling the extension stops future refresh passes; it does not revoke
the extension's own external login. Source: [extension.rs](../windows/quotascope-core/src/extension.rs).
