CREATE TABLE "user" (
	"id" INTEGER NOT NULL UNIQUE,
	"email" VARCHAR,
	"_created" TIMESTAMPTZ,
	PRIMARY KEY("id")
);


CREATE TABLE "robot" (
	"id" INTEGER NOT NULL UNIQUE,
	"name" VARCHAR,
	"_created" TIMESTAMPTZ,
	"owner_id" INTEGER,
	PRIMARY KEY("id")
);


CREATE TABLE "part" (
	"id" INTEGER NOT NULL UNIQUE,
	"name" VARCHAR,
	"cost" NUMERIC,
	PRIMARY KEY("id")
);


CREATE TABLE "robot_parts" (
	"id" INTEGER NOT NULL UNIQUE,
	"robot_id" INTEGER,
	"part_id" INTEGER,
	"part_quantity" DECIMAL,
	PRIMARY KEY("id")
);


ALTER TABLE "robot"
ADD FOREIGN KEY("owner_id") REFERENCES "user"("id")
ON UPDATE NO ACTION ON DELETE NO ACTION;
ALTER TABLE "robot_parts"
ADD FOREIGN KEY("robot_id") REFERENCES "robot"("id")
ON UPDATE NO ACTION ON DELETE NO ACTION;
ALTER TABLE "robot_parts"
ADD FOREIGN KEY("part_id") REFERENCES "part"("id")
ON UPDATE NO ACTION ON DELETE NO ACTION;


-- Populate "user" table
INSERT INTO "user" ("id", "email", "_created") VALUES
(1, 'paul.atreides@arrakis.com', CURRENT_TIMESTAMP),
(2, 'leto.atreides@caladan.com', CURRENT_TIMESTAMP),
(3, 'baron.harkonnen@giedi.com', CURRENT_TIMESTAMP);

-- Populate "robot" table
INSERT INTO "robot" ("id", "name", "_created", "owner_id") VALUES
(1, 'Hunter-Seeker', CURRENT_TIMESTAMP, 3),
(2, 'Harvester', CURRENT_TIMESTAMP, 1),
(3, 'Ornithopter', CURRENT_TIMESTAMP, 2);

-- Populate "part" table
INSERT INTO "part" ("id", "name", "cost") VALUES
(1, 'Suspensor', 1000.00),
(2, 'Holtzman Shield', 5000.00),
(3, 'Spice Sensor', 2500.00);

-- Populate "robot_parts" table
INSERT INTO "robot_parts" ("id", "robot_id", "part_id", "part_quantity") VALUES
(1, 1, 1, 2),
(2, 1, 2, 1),
(3, 2, 1, 4),
(4, 2, 3, 2),
(5, 3, 1, 6),
(6, 3, 2, 1),
(7, 3, 3, 1);
-- Additional data for "user" table
INSERT INTO "user" ("id", "email", "_created") VALUES
(4, 'lady.jessica@bene.gesserit', CURRENT_TIMESTAMP),
(5, 'gurney.halleck@atreides.com', CURRENT_TIMESTAMP),
(6, 'duncan.idaho@swordmaster.com', CURRENT_TIMESTAMP),
(7, 'thufir.hawat@mentat.com', CURRENT_TIMESTAMP),
(8, 'liet.kynes@fremen.com', CURRENT_TIMESTAMP),
(9, 'stilgar@sietch.tabr', CURRENT_TIMESTAMP),
(10, 'chani@fremen.warrior', CURRENT_TIMESTAMP),
(11, 'alia.atreides@dune.com', CURRENT_TIMESTAMP),
(12, 'feyd.rautha@harkonnen.com', CURRENT_TIMESTAMP),
(13, 'piter.devries@twisted.mentat', CURRENT_TIMESTAMP),
(14, 'shaddam.iv@padishah.emperor', CURRENT_TIMESTAMP),
(15, 'irulan.corrino@imperial.princess', CURRENT_TIMESTAMP);

-- Additional data for "robot" table
INSERT INTO "robot" ("id", "name", "_created", "owner_id") VALUES
(4, 'Spice Harvester', CURRENT_TIMESTAMP, 4),
(5, 'Sandcrawler', CURRENT_TIMESTAMP, 5),
(6, 'Water Extractor', CURRENT_TIMESTAMP, 6),
(7, 'Stillsuit Recycler', CURRENT_TIMESTAMP, 7),
(8, 'Thumper', CURRENT_TIMESTAMP, 8),
(9, 'Weirding Module', CURRENT_TIMESTAMP, 9),
(10, 'Ornithopter Scout', CURRENT_TIMESTAMP, 10),
(11, 'Guild Navigator Aid', CURRENT_TIMESTAMP, 11),
(12, 'Poison Snooper', CURRENT_TIMESTAMP, 12),
(13, 'Sardaukar Training Bot', CURRENT_TIMESTAMP, 13),
(14, 'Imperial Courier', CURRENT_TIMESTAMP, 14),
(15, 'Bene Gesserit Truth Sayer', CURRENT_TIMESTAMP, 15);

-- Additional data for "part" table
INSERT INTO "part" ("id", "name", "cost") VALUES
(4, 'Lasgun', 3000.00),
(5, 'Crysknife', 1500.00),
(6, 'Stillsuit', 2000.00),
(7, 'Maker Hook', 500.00),
(8, 'Fremkit', 1000.00),
(9, 'Poison Tooth', 750.00),
(10, 'Gom Jabbar', 5000.00),
(11, 'Distrans', 1200.00),
(12, 'Jubba Cloak', 800.00),
(13, 'Paracompass', 1800.00),
(14, 'Portable Stilltent', 2200.00),
(15, 'Spice Melange Detector', 4000.00);

-- Additional data for "robot_parts" table
INSERT INTO "robot_parts" ("id", "robot_id", "part_id", "part_quantity") VALUES
(8, 4, 1, 8),
(9, 4, 3, 4),
(10, 4, 6, 2),
(11, 5, 1, 6),
(12, 5, 4, 2),
(13, 5, 8, 1),
(14, 6, 1, 4),
(15, 6, 6, 3),
(16, 6, 15, 1),
(17, 7, 6, 5),
(18, 7, 8, 2),
(19, 7, 14, 1),
(20, 8, 7, 3),
(21, 8, 11, 1),
(22, 9, 2, 2),
(23, 9, 4, 1),
(24, 9, 10, 1),
(25, 10, 1, 4),
(26, 10, 2, 1),
(27, 10, 13, 2),
(28, 11, 3, 3),
(29, 11, 15, 2),
(30, 12, 9, 1),
(31, 12, 12, 2),
(32, 13, 4, 5),
(33, 13, 5, 10),
(34, 14, 1, 2),
(35, 14, 2, 1),
(36, 14, 11, 3),
(37, 15, 3, 1),
(38, 15, 10, 1),
(39, 15, 15, 1);
