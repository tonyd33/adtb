---
title: Gel constraints
links:
  - {rel: prior-art, to: prior/gel}
---
`constraint exclusive on ((.x, .y))`, `constraint expression on (...)`
plus value constraints. Expression constraints see only single
properties and links of one object, never paths of more than one hop.
A supertype's constraint applies across all subtypes jointly;
`delegated` applies it per subtype.
https://docs.geldata.com/reference/datamodel/constraints
