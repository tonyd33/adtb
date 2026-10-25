---
title: Class-hierarchy index for object-oriented databases
---
W. Kim, K.-C. Kim, A. Dale, "Indexing Techniques for Object-Oriented
Databases", in Kim and Lochovsky (eds.), Object-Oriented Concepts,
Databases, and Applications, Addison-Wesley, 1989. One B+-tree over a
class and all its subclasses, grouping per key the OIDs of each class.
Each object belongs to one class, so the indexed extent is a disjoint
union. Followed by H-trees (Low, Ooi, Lu, SIGMOD 1992), hcC-trees
(Sreenath, Seshadri, VLDB 1994) and class-division (Ramaswamy,
Kanellakis, Brown CS-95-15, which reports most systems use CH). Leaf
layout and page numbers unverified.
