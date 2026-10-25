# Haskell lens library

kind: language
home: https://hackage.haskell.org/package/lens

## Prisms

- A Prism is "a Traversal that can also be turned around with `re` to obtain a Getter in the opposite direction". [V 2026-10-25] https://hackage-content.haskell.org/package/lens-5.3.6/docs/Control-Lens-Prism.html
- Setting through a prism that does not match leaves the value unchanged: `Right 5 & _Left .~ 10` gives `Right 5`. [V 2026-10-25] same page
