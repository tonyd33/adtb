CREATE TABLE counter(id INTEGER PRIMARY KEY, value INTEGER NOT NULL) STRICT;
CREATE TABLE campaign(id INTEGER PRIMARY KEY, title TEXT NOT NULL) STRICT;
CREATE TABLE entity(
 id INTEGER PRIMARY KEY, campaign INTEGER NOT NULL REFERENCES campaign(id),
 name TEXT NOT NULL, tag TEXT NOT NULL CHECK(tag IN ('Party','Npc')),
 UNIQUE(campaign,name), UNIQUE(id,tag,campaign)
) STRICT;
CREATE INDEX entity_campaign ON entity(campaign);
CREATE TABLE session(
 id INTEGER PRIMARY KEY, campaign INTEGER NOT NULL REFERENCES campaign(id),
 party_id INTEGER NOT NULL, party_tag TEXT NOT NULL CHECK(party_tag='Party'),
 number INTEGER NOT NULL, title TEXT NOT NULL, prep TEXT NOT NULL,
 tag TEXT NOT NULL CHECK(tag IN ('Planned','Live','Played')),
 planned_for INTEGER, tick INTEGER, day INTEGER, log TEXT, recap TEXT,
 UNIQUE(campaign,number),
 FOREIGN KEY(party_id,party_tag,campaign) REFERENCES entity(id,tag,campaign),
 CHECK(
  (tag='Planned' AND tick IS NULL AND day IS NULL AND log IS NULL AND recap IS NULL)
  OR (tag='Live' AND planned_for IS NULL AND tick IS NOT NULL AND day IS NULL AND log IS NULL AND recap IS NULL)
  OR (tag='Played' AND planned_for IS NULL AND tick IS NULL AND day IS NOT NULL AND log IS NOT NULL AND recap IS NOT NULL)
 )
) STRICT;
CREATE INDEX session_party ON session(party_id,party_tag,campaign);
CREATE UNIQUE INDEX one_live ON session(campaign) WHERE tag='Live';
CREATE TRIGGER played_stays_played BEFORE UPDATE ON session
 WHEN OLD.tag='Played' AND NEW.tag IN ('Planned','Live')
 BEGIN SELECT RAISE(ABORT,'Played stays Played'); END;
