# Security policy

## Supported versions

Security fixes are made on `main` and released in the next pgorm version. Only
the latest published version is supported; older versions should be upgraded
before reporting a version-specific issue.

## Report a vulnerability

Please do not open a public issue for a suspected vulnerability. Use
[GitHub's private vulnerability reporting](https://github.com/necessary-nu/pgorm/security/advisories/new)
so maintainers can investigate and coordinate a fix without exposing users.
If that channel is unavailable, email `brendan@necessary.nu` with
`[pgorm security]` in the subject.

Include, where possible:

- the affected version or commit;
- the affected crate or binding;
- impact and the conditions required to exploit the issue;
- a minimal reproduction or proof of concept; and
- any suggested mitigation or fix.

Do not include secrets or personal data. Use a test system and stop if testing
could affect other users or data.

We aim to acknowledge a report within three business days and provide an
initial triage update within seven business days. Remediation and disclosure
timing depends on severity and release coordination. Please keep the report
private until a fix or agreed mitigation is available. Credit is offered when
requested and appropriate.

Good-faith research that follows this policy, avoids privacy violations and
service disruption, and gives maintainers a reasonable opportunity to fix the
issue will not be treated as abusive activity by this project.

## Scope

Reports about pgorm's Rust crates, Python package, JavaScript package, code
generation, SQL construction or execution, and project-maintained security
automation are in scope. Vulnerabilities in an upstream dependency are also
welcome when they materially affect pgorm; include the upstream advisory or
report when one exists.

General support questions, correctness bugs without a security impact, and
findings that require a deliberately unsafe use of raw SQL belong in the public
issue tracker instead.
