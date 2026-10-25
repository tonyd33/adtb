# Ruby on Rails (Active Record)

kind: system
home: https://guides.rubyonrails.org/

## Associations

- Polymorphic belongs_to stores imageable_type (class name) and imageable_id; composite index recommended; no FK constraint possible. [V 2026-10-25] https://guides.rubyonrails.org/association_basics.html (no-FK point [U])
- Single table inheritance uses a type column holding the subclass name. [V 2026-10-25] same
