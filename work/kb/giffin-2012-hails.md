# Hails (Giffin et al., OSDI 2012)

kind: paper
home: https://www.usenix.org/system/files/conference/osdi12/osdi12-final-35.pdf

## Model

- Web framework adding mandatory access control and a declarative policy language to MVC; policy modules (MPs) own a database and hold a dedicated principal's privileges; untrusted view-controllers (VCs) are confined. Labels are DC labels (readers, writers). [V 2026-10-25] pdftotext of paper, sections 1-2
- Static labels on database and collection; the collection label also protects key values: "a computation that can read from a collection can also read all the key values". [V 2026-10-25] section 2.3
- Document and field labels are computed by pure functions of the document itself ("the authoritative source for who should access data resides in the data itself"); applied on every insert. [V 2026-10-25] section 2.3.1-2.3.2
- Concurrent confinement in LIO claimed sound against termination and timing covert channels; OS and network assumed not attacker-controlled. [V 2026-10-25] section 3
