# Product name and listing review (#32)

Reviewed 2026-10-16 for keeping **ElevenClock-rs**, as requested by the owner.
Distribution scope confirmed by the owner: **GitHub releases first; winget later**.
Microsoft Store publication/reservation is outside the current release plan.
Source: PR #48, branch `codex/apache2-licensing-and-artwork`.
Status: **technical identities and listing draft prepared; name review incomplete**.
No name reservation, trademark registration, winget submission or Store submission
was made. Public availability checks do not reserve a name or establish rights.

## Existing use and package separation

| Surface | Observed upstream use | This project's identity |
| --- | --- | --- |
| Product name | ElevenClock, maintained by Martí Climent | ElevenClock-rs, maintained separately by tjorim |
| Microsoft Store | [ElevenClock, product XP89HZ8SVWTT0M](https://apps.microsoft.com/detail/XP89HZ8SVWTT0M) | No Store ID or name reservation established; do not reuse upstream's product ID |
| winget | `SomePythonThings.ElevenClock`, publisher Martí Climent, moniker `elevenclock` | Proposed `tjorim.ElevenClockRs`, publisher ElevenClock-rs contributors, proposed moniker `elevenclock-rs` |
| Installer AppId | `{D62480B8-71F1-48CE-BEEC-9D3E172C87B5}` | `{97911B67-52D7-4D9B-B833-29303BC540A7}` |
| Start menu AppUserModelID | Not reused | Existing `tjorim.ElevenClockRs` |
| Executable | Legacy ElevenClock | `elevenclock-rs.exe` |
| Install/data directories | Legacy paths retained separately | `%LOCALAPPDATA%\Programs\ElevenClock-rs` and `%LOCALAPPDATA%\ElevenClock-rs` |
| Login startup value | `elevenClock` | Existing `ElevenClockRust`; preserve this established ownership contract |

The [upstream winget manifest](https://github.com/microsoft/winget-pkgs/blob/master/manifests/s/SomePythonThings/ElevenClock/4.4.1.1/SomePythonThings.ElevenClock.locale.en-US.yaml)
was read directly from Microsoft's repository. It confirms the upstream ID,
publisher, product name, moniker and project URL; its tags also identify the Store
product. The public Store search returned the existing ElevenClock product page,
but the page content requires JavaScript; its complete current metadata was not
independently extracted in this review.

A direct GitHub Contents API lookup of
`manifests/t/tjorim/ElevenClockRs` in `microsoft/winget-pkgs` returned 404 on the
review date. This establishes only that no manifest existed at that exact path
at that time. It does not reserve an identifier, check pending submissions or
establish acceptance. Microsoft's [manifest guidance](https://learn.microsoft.com/en-us/windows/package-manager/package/manifest)
uses `Publisher.Package` identifiers; the proposed ID follows that form and is
distinct from upstream's. An AppUserModelID is not a winget registration or a
Microsoft Store identity, even when the strings match.

## Trademark search performed and its limits

Read-only searches were performed in the official USPTO search UI and EUIPN's
TMview UI. No class or territory restriction was added; default search behavior
and status selections were retained.

| Source and query | Observed result | Interpretation |
| --- | --- | --- |
| [USPTO](https://tmsearch.uspto.gov/), Wordmark `ElevenClock` | No results found; live/dead counts zero | No result for that basic query, not a finding that the name is free to use |
| USPTO, Wordmark `"Eleven Clock"` | 2,103 results; first visible result was cancelled `ELEVEN O' CLOCK`, serial 76345392, tea | The basic search broadens/tokenizes the query; these results were not exhaustively reviewed |
| USPTO, Wordmark `ElevenClock-rs` | 985 results | Broad/tokenized result set, not 985 exact-name conflicts; not exhaustively reviewed |
| [TMview Contains `ElevenClock`](https://www.tmdn.org/tmview/#/tmview/results?page=1&pageSize=30&criteria=C&basicSearch=ElevenClock) | No rows found | No result for this query in the aggregator; not registry clearance |
| [TMview Contains `Eleven Clock`](https://www.tmdn.org/tmview/#/tmview/results?page=1&pageSize=30&criteria=C&basicSearch=Eleven%20Clock) | Multiple similar/broad matches; first page inspected | Includes registered `ELEVEN O'CLOCK` for class 30, EU application 002288454, and other marks; no full similarity assessment completed |
| [Direct BOIP register](https://www.boip.int/en/trademarks-register) | reCAPTCHA blocked access before searching | No direct Benelux result obtained; no challenge was solved or bypassed |

TMview itself states that it is not an official register and its information has
no legal effect. Indexed web searches for ElevenClock on USPTO/BOIP/EUIPO/WIPO
domains did not supply an authoritative software-name registration result; that
absence is not a substitute for register searches. No WIPO-specific search or
exhaustive national-register/similar-spelling search was completed.

The concrete existing same-category use is upstream's Windows taskbar-clock
product. Keeping its full name with `-rs` can still suggest affiliation or an
official successor; the suffix alone does not settle that question. This is a
review inference, not a finding of infringement. USPTO explains that
[similarity and related goods/services matter](https://www.uspto.gov/trademarks/search/likelihood-confusion).
The Apache license for our implementation does not grant rights to someone
else's product name.

## Listing copy and intended identity

Use the following as a reviewable draft, subject to actual release validation:

- **Display name:** ElevenClock-rs.
- **Publisher:** ElevenClock-rs contributors. **Maintainer:** tjorim.
- **Short description:** A customizable native clock for the Windows 11 taskbar.
- **Project URL:** https://github.com/tjorim/ElevenClock-rs
- **Support URL:** https://github.com/tjorim/ElevenClock-rs/issues
- **License:** Apache-2.0 for the original implementation and artwork. Retained
  legacy source and third-party notices keep their own terms.
- **Relationship statement:** An independent Rust implementation maintained by
  tjorim. Affiliation with or endorsement by the original ElevenClock project is
  not claimed.
- **Description:** Choose a date/time format or ISO week-date preset, adjust the
  font and clock theme, and manage login startup through the app's native Settings window.
  Closing the app's Settings window keeps the clock running; Exit restores the
  native clock. This is a separate application from the original Python/Qt
  ElevenClock. Do not run both overlay applications simultaneously.

Current development candidates are unsigned and have no supported Windows build
matrix yet. Do not publish this draft as a supported release listing or claim
unverified monitor/OS compatibility. Use the original icon and current-app
screenshots only, not `legacy/media/` imagery. Do not use upstream's Store badges,
package IDs, publisher identity or `elevenclock` moniker for this app.

## Remaining work before the name gate passes

- [ ] Complete and record a focused similar-name/software-goods review for the
  intended distribution territories, including the direct Benelux search that
  could not be performed here. Record the disposition of relevant hits, not just
  search counts. The current searches do not establish clearance.
- [ ] Record the owner's decision on keeping this name in light of upstream's
  same-category use. Written upstream agreement on the name is one practical way
  to address the relationship; no message or agreement is claimed by this task.
  If relying on another basis, record that basis and any qualified advice obtained.
Deferred channel work (not a prerequisite for a GitHub-only release):

- [ ] If Microsoft Store distribution is later chosen, check **ElevenClock-rs** in the
  owner's Partner Center account and record the result. A public Store search
  cannot reveal other developers' unpublished reservations. Reservation/submission
  is a separate owner-authorized action, not performed in this review.
- [ ] Before winget submission, recheck the proposed ID and moniker, use the
  actual released installer URL/SHA256/version, verify silent per-user install and
  uninstall detection, and validate the manifest against that artifact. No fake
  installer URL/hash or submission-ready manifest is created before a release.

Microsoft's [MSI/EXE name-reservation instructions](https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msi/reserve-your-apps-name)
require a unique name and a Partner Center availability check; a name can be
reserved even when no public listing exists. They recommend distinctive names
and avoiding trailing differentiators. This is a reason to check acceptance of
`ElevenClock-rs`, not a claim that the hyphenated name is automatically prohibited.
The current MSI/EXE route does not require inventing a Store product ID or MSIX
identity here; retain the native installer identity until a channel decision.

`packaging/release-gates.json` remains unchanged with `licensing.passed = false`
for the outstanding name decision, not because a Store listing is needed.
Windows acceptance and clean-account gates remain separate. This record completes the repository identity inventory and a
preliminary listing/search pass; it does not claim complete legal or Store review.
