# Hibernate

kind: system
home: https://docs.hibernate.org/orm/

## Inheritance mapping

- SINGLE_TABLE called most efficient; subclass columns must be nullable. TABLE_PER_CLASS reads polymorphically with union, no identity generator. JOINED needs joins. [S 2026-10-25] https://docs.hibernate.org/orm/6.4/userguide/html_single/Hibernate_User_Guide.html
