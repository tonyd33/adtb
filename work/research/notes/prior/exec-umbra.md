---
title: Umbra adaptive compilation
---
Kohn, Leis, Neumann, ICDE 2018: start every pipeline in a bytecode
interpreter, extrapolate progress per morsel, compile when it pays;
a 1 ms query took 54 ms to compile in HyPer.
https://db.in.tum.de/~leis/papers/adaptiveexecution.pdf
Neumann, Freitag, CIDR 2020: Umbra IR, pipelines as steps, 3.0x over
HyPer on JOB. https://www.cidrdb.org/cidr2020/papers/p29-neumann-cidr20.pdf
Engelke, Schwarz, CGO 2024: Cranelift only 20-35% faster to compile
than LLVM; Umbra's single-pass emitter 16x faster than Cranelift.
https://home.cit.tum.de/~engelke/pubs/2403-cgo.pdf
