# Jenkins LSP completion context

The LSP uses a bounded lexical scan of the Jenkinsfile prefix up to the completion cursor to determine the active Declarative Pipeline blocks. It tracks all opening braces (including unclassified Groovy closures) so closing a nested block does not accidentally close its parent.

The scanner recognizes these Declarative block names: \`pipeline\`, \`stages\`, \`stage\`, \`steps\`, \`post\`, \`script\`, \`agent\`, \`environment\`, \`options\`, \`parameters\`, \`triggers\`, \`tools\`, \`when\`, \`parallel\`, \`matrix\`, \`axes\`, \`excludes\`, and \`input\`.

It skips line comments, block comments, single- and double-quoted strings, and triple-single/triple-double-quoted strings. Escaped delimiters are honored. Incomplete documents and unterminated comments/strings are handled conservatively; unclosed blocks remain active until a matching brace appears. Context scanning is capped at 1 MiB before the cursor. Larger prefixes fall back to the safe non-Declarative completion context.

This is a context heuristic, not a complete Groovy parser. Groovy slashy and dollar-slashy strings, GString interpolation as executable code, annotations/macros that generate structure, and unusual block headers are not fully parsed. The scanner does not execute source or perform unbounded parsing. Add regression cases before extending the recognized syntax.
