# Jenkins LSP completion context

The LSP uses a bounded lexical scan of the Jenkinsfile prefix up to the completion cursor to determine the active Declarative Pipeline blocks. It tracks all opening braces (including unclassified Groovy closures) so closing a nested block does not accidentally close its parent.

The scanner recognizes these Declarative block names: \`pipeline\`, \`stages\`, \`stage\`, \`steps\`, \`post\`, \`script\`, \`agent\`, \`environment\`, \`options\`, \`parameters\`, \`triggers\`, \`tools\`, \`when\`, \`parallel\`, \`matrix\`, \`axes\`, \`excludes\`, and \`input\`.

It skips line comments, block comments, single- and double-quoted strings, triple-single/triple-double-quoted strings, slashy strings, and dollar-slashy strings. Escaped delimiters and dollar-slashy escape sequences are honored. String interpolation is skipped as literal content so embedded braces and directive-like names cannot affect the Declarative stack. Incomplete documents and unterminated comments/strings are handled conservatively; unclosed blocks remain active until a matching brace appears. Context scanning is capped at 1 MiB before the cursor. Larger prefixes fall back to the safe non-Declarative completion context.

This is a context heuristic, not a complete Groovy parser. Block names are considered only at statement starts outside argument lists, map/list literals, ordinary Groovy closures, and `script` bodies; qualified method calls, assignments, and map keys do not open Declarative contexts. GString interpolation is intentionally treated as opaque literal content. Annotations/macros that generate structure and unusual block headers are not fully parsed. The scanner does not execute source or perform unbounded parsing. Add regression cases before extending the recognized syntax.


## Collection completeness

The LSP preserves the package's collection status separately from symbol confidence. The custom request `jenkinsKnowledge/collection` returns the manifest's package-level completeness and per-source outcomes, including permission-denied, unavailable, failed, skipped-by-profile, and not-configured states. `jenkinsKnowledge/coverage` remains the API-surface coverage view. Older packages without the additive manifest collection object return an explicit `unknown` status; absence is never upgraded to complete.
