# Code-signing trust anchors

Microsoft's public root certificates that Windows trusts for code signing and
time stamping. They are not part of the Mozilla TLS root set that `door-tls`
ships, so programs signed through Microsoft's PKI (Windows components, Visual
Studio tools, Azure Trusted Signing) would otherwise not verify.

Exported unchanged as DER from a Windows 11 Pro 26H2 (build 26300) machine's
`Cert:\LocalMachine\Root` store on 2026-11-02 (read-only export). Root
certificates are public data; Microsoft also publishes them at
<https://www.microsoft.com/pkiops/docs/repository.htm>.

| File | Subject CN | Expires | SHA-1 thumbprint |
|---|---|---|---|
| `microsoft-root-2010.der` | Microsoft Root Certificate Authority 2010 | 2035-06-23 | `3B1EFD3A66EA28B16697394703A72CA340A05BD5` |
| `microsoft-root-2011.der` | Microsoft Root Certificate Authority 2011 | 2036-03-22 | `8F43288AD272F3103B6FB1428485EA3014C0BCFE` |
| `microsoft-time-stamp-root-2014.der` | Microsoft Time Stamp Root Certificate Authority 2014 | 2039-10-22 | `0119E81BE9A14CD8E22F40AC118C687ECBA3F4D8` |
| `microsoft-ecc-ts-root-2018.der` | Microsoft ECC TS Root Certificate Authority 2018 | 2043-02-27 | `31F9FC8BA3805986B721EA7295C65B3A44534274` |
| `microsoft-ecc-product-root-2018.der` | Microsoft ECC Product Root Certificate Authority 2018 | 2043-02-27 | `06F1AA330B927B753A40E68CDF22E34BCBEF3352` |
| `microsoft-identity-verification-root-2020.der` | Microsoft Identity Verification Root Certificate Authority 2020 | 2045-04-16 | `F40042E2E5F7E8EF8189FED15519AECE42C3BFA2` |

Public commercial code-signing CAs (DigiCert, Sectigo, GlobalSign, Entrust,
SSL.com, Certum) come from the Mozilla set through `door_tls::verify::mozilla_roots`.

Public CAs' dedicated code-signing roots and the legacy roots older
time-stamping authorities chain to, exported the same way:

| File | Subject CN | Expires | SHA-1 thumbprint |
|---|---|---|---|
| `sectigo-code-signing-root-r46.der` | Sectigo Public Code Signing Root R46 | 2046-03-22 | `CCBBF9E1485AF63CE47ABF8E9E648C2504FC319D` |
| `sectigo-code-signing-root-e46.der` | Sectigo Public Code Signing Root E46 | 2046-03-22 | `BBEF5C4C11489770F586FB307D143291307F119A` |
| `digicert-cs-rsa4096-root-g5.der` | DigiCert CS RSA4096 Root G5 | 2046-01-15 | `5EEED86FA37C675230642F55C84DDBF67CD33C80` |
| `globalsign-code-signing-root-r45.der` | GlobalSign Code Signing Root R45 | 2045-03-18 | `4EFC31460C619ECAE59C1BCE2C008036D94C84B8` |
| `verisign-universal-root.der` | VeriSign Universal Root Certification Authority | 2037-12-02 | `3679CA35668772304D30A5FB873B0FA77BB70D54` |
| `verisign-class3-g5.der` | VeriSign Class 3 Public Primary Certification Authority - G5 | 2036-07-17 | `4EB6D578499B1CCF5F581EAD56BE3D9B6744A5E5` |
| `thawte-primary-root.der` | thawte Primary Root CA | 2036-07-17 | `91C6D6EE3E8AC86384E548C299295C756C817B81` |

The legacy roots carry SHA-1 self-signatures; only their names and keys are
used as anchors. `door_tls::verify::signature` refuses SHA-1 certificate
signatures, so a chain still needs SHA-2 certificates below the root.
