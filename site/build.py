#!/usr/bin/env python3
"""Build the website into site/dist from site/src and the catalog.

    cargo build --release
    ./target/release/handrail __catalog-json > /tmp/catalog.json
    python3 site/build.py /tmp/catalog.json

The pack and profile sections are generated from the catalog compiled into the binary, so
the site can never describe packs differently from what `handrail` installs.
"""
import html
import json
import shutil
import sys
from pathlib import Path

SRC = Path(__file__).parent / "src"
DIST = Path(__file__).parent / "dist"
DOMAIN = "handrail.bitey.ai"

CATEGORY_ORDER = ["security", "privacy", "audit"]
RECOMMENDED = "baseline"


def e(s):
    return html.escape(str(s), quote=True)


def enforcement_badge(level):
    cls = {"enforced": "b-enforced", "partial": "b-partial", "advisory": "b-advisory"}.get(level, "b-neutral")
    return f'<span class="badge {cls}">{e(level)}</span>'


def pack_card(p):
    cc = p["targets"].get("claude-code", {})
    level = cc.get("enforcement", "unsupported")
    minv = cc.get("min_version")
    lists = []
    if p["protects"]:
        lists.append("<dt>Protects against</dt><dd><ul>" + "".join(f"<li>{e(x)}</li>" for x in p["protects"]) + "</ul></dd>")
    if p["tradeoffs"]:
        lists.append("<dt>Tradeoffs</dt><dd><ul>" + "".join(f"<li>{e(x)}</li>" for x in p["tradeoffs"]) + "</ul></dd>")
    lists.append(f"<dt>Limits</dt><dd>{e(p['limits'])}</dd>")
    if minv:
        lists.append(f"<dt>Requires</dt><dd>Claude Code {e(minv)} or later</dd>")
    return f"""      <article class="pack" data-category="{e(p['category'])}">
        <div class="pack-top">{enforcement_badge(level)}<span class="badge b-neutral">{e(p['tier'])} tier</span><span class="pack-id">{e(p['id'])}</span></div>
        <h3>{e(p['title'])}</h3>
        <p>{e(p['summary'])}</p>
        <details><summary>Details</summary><dl>{''.join(lists)}</dl></details>
        <div class="cmd"><code>handrail enable {e(p['id'])}</code></div>
      </article>"""


def filters(packs):
    counts = {}
    for p in packs:
        counts[p["category"]] = counts.get(p["category"], 0) + 1
    cats = sorted(counts, key=lambda c: (CATEGORY_ORDER.index(c) if c in CATEGORY_ORDER else 99, c))
    chips = [f'<button class="chip" type="button" data-filter="all" aria-pressed="true">All<span class="n">{len(packs)}</span></button>']
    chips += [f'<button class="chip" type="button" data-filter="{e(c)}" aria-pressed="false">{e(c.capitalize())}<span class="n">{counts[c]}</span></button>' for c in cats]
    return '<div class="filters" role="group" aria-label="Filter packs by category">' + "".join(chips) + "</div>"


def profile_cards(profiles):
    order = {"baseline": 0, "strict": 1, "paranoid": 2}
    profiles = sorted(profiles, key=lambda p: order.get(p["name"], 9))
    out, prev = [], set()
    for pr in profiles:
        rec = pr["name"] == RECOMMENDED
        members = "".join(f'<span class="{"new" if prev and m not in prev else ""}">{e(m)}</span>' for m in pr["packs"])
        tag = '<span class="badge b-enforced" style="font-size:11px">recommended</span>' if rec else ""
        out.append(f"""      <div class="profile{' rec' if rec else ''}">
        <div class="name">{e(pr['name'])} {tag}</div>
        <p>{e(pr['description'])}</p>
        <div class="members">{members}</div>
        <pre class="snippet">handrail use {e(pr['name'])}</pre>
      </div>""")
        prev = set(pr["packs"])
    return "\n".join(out)


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    catalog = json.loads(Path(sys.argv[1]).read_text())
    packs = sorted(catalog["packs"], key=lambda p: (CATEGORY_ORDER.index(p["category"]) if p["category"] in CATEGORY_ORDER else 99, p["id"]))

    if DIST.exists():
        shutil.rmtree(DIST)
    DIST.mkdir(parents=True)

    header = (SRC / "_header.html").read_text()
    footer = (SRC / "_footer.html").read_text()

    def chrome(html_text, page):
        """One header and footer for every page, so navigation can never drift apart.
        The current page's link is marked for screen readers and styling."""
        h = header.replace(f'data-page="{page}"', f'data-page="{page}" aria-current="page"')
        return html_text.replace("{{HEADER}}", h).replace("{{FOOTER}}", footer)

    index = chrome((SRC / "index.html").read_text(), "home")
    for key, value in {
        "{{VERSION}}": e(catalog["version"]),
        "{{PACK_COUNT}}": str(len(packs)),
        "{{PROFILE_COUNT}}": str(len(catalog["profiles"])),
        "{{PACK_FILTERS}}": filters(packs),
        "{{PACK_CARDS}}": "\n".join(pack_card(p) for p in packs),
        "{{PROFILES}}": profile_cards(catalog["profiles"]),
    }.items():
        if key not in index:
            sys.exit(f"template is missing {key}")
        index = index.replace(key, value)
    leftover = [t for t in ("{{",) if t in index]
    if leftover:
        sys.exit("unreplaced placeholder in index.html")
    (DIST / "index.html").write_text(index)

    docs = chrome((SRC / "docs.html").read_text(), "docs")
    if "{{" in docs:
        sys.exit("unreplaced placeholder in docs.html")
    (DIST / "docs.html").write_text(docs)
    for name in ("style.css", "install.sh"):
        shutil.copy2(SRC / name, DIST / name)
    (DIST / "CNAME").write_text(DOMAIN + "\n")
    # GitHub Pages: serve files as-is (no Jekyll processing)
    (DIST / ".nojekyll").write_text("")
    print(f"built {DIST} ({len(packs)} packs, {len(catalog['profiles'])} profiles)")


if __name__ == "__main__":
    main()
