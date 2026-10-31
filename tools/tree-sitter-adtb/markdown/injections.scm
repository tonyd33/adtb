; Install as an injections query of a markdown grammar: highlights `adt` code blocks.
((fenced_code_block
  (info_string (language) @_language)
  (code_fence_content) @injection.content)
  (#eq? @_language "adt")
  (#set! injection.language "adtb"))
