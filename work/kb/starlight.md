# Astro Starlight

kind: system
home: https://starlight.astro.build/

## Source integration

- Every Starlight page requires title metadata. Its default docsLoader reads src/content/docs, while Astro's glob loader supports files anywhere on disk and custom loaders can supply metadata. [V 2026-10-27] https://starlight.astro.build/reference/frontmatter/#title-required https://starlight.astro.build/reference/configuration/#docsloader https://docs.astro.build/en/guides/content-collections/#the-glob-loader
- Pagefind search is enabled by default for prerendered sites. [V 2026-10-27] https://starlight.astro.build/reference/configuration/#pagefind
- Registry latest at research time: astro 7.4.2 and @astrojs/starlight 0.42.7; Starlight peers astro ^7.2.10 and @astrojs/markdown-remark ^7.3.0; Astro requires Node >=22.23.4. Production must pin versions, not resolve latest dynamically. [V 2026-10-27] https://registry.npmjs.org/astro https://registry.npmjs.org/@astrojs%2fstarlight
- adtb prototype built all 57 existing pages without front matter edits using a glob loader wrapper that derives title metadata from the first H1, a remark link rewrite to source-relative route URLs, and a Shiki TextMate grammar. Cold build 6.94 s, warm 3.34 s; 354 installed dependency packages (prototype versions above). Adapted output passed 3,951 internal links including 3,592 fragments. Chromium verified cross-section Pagefind results, numbered anchors and highlighted adt spans. Raw H1 remained duplicated by Starlight's page title; prototype favicon was missing. These are local observations. [V 2026-10-27] Local evidence: target/docs-prototype/{astro.config.mjs,src/content.config.mjs,starlight-build.log,browser-check.log,dist-adapted-links.txt} in docs-site-research workspace.
