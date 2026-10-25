---
title: Prisms as updates into a sum
---
A prism focuses one constructor of a sum; setting through a prism on a
value of another constructor leaves it unchanged
(`Right 5 & _Left .~ 10` is `Right 5`).
https://hackage-content.haskell.org/package/lens-5.3.6/docs/Control-Lens-Prism.html
