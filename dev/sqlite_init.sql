-- transformed from the mysql_init.sql data to sqlite3 by claude 3 haiku
CREATE TABLE `user` (
    `id` INTEGER PRIMARY KEY AUTOINCREMENT,
    `email` VARCHAR(255),
    `nicknames` JSON,
    `age` SMALLINT,
    `height` FLOAT,
    `is_active` BOOLEAN,
    `last_login` TIMESTAMP,
    `_created` TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE `robot` (
    `id` INTEGER PRIMARY KEY AUTOINCREMENT, 
    `name` VARCHAR(100),
    `manufacture_date` DATE,
    `specifications` JSON,
    `serial_number` CHAR(36),
    `_created` TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    `owner_id` INTEGER NULL,
    FOREIGN KEY (`owner_id`) REFERENCES `user`(`id`) ON DELETE SET NULL
);

CREATE TABLE `part` (
    `id` INTEGER PRIMARY KEY AUTOINCREMENT,
    `name` VARCHAR(100),
    `cost` DECIMAL(10,2),
    `weight` DOUBLE,
    `dimensions` BLOB,
    `previous_prices` JSON,
    `description` TEXT
);

CREATE TABLE `robot_parts` (
    `id` INTEGER PRIMARY KEY AUTOINCREMENT,
    `robot_id` INTEGER NULL,
    `part_id` INTEGER NULL,
    `part_quantity` INTEGER,
    `installation_date` DATE,
    FOREIGN KEY (`robot_id`) REFERENCES `robot`(`id`) ON DELETE SET NULL,
    FOREIGN KEY (`part_id`) REFERENCES `part`(`id`) ON DELETE SET NULL
);

CREATE TABLE `sensor_data` (
    `id` INTEGER PRIMARY KEY AUTOINCREMENT,
    `robot_id` INTEGER,
    `temperature` FLOAT,
    `humidity` FLOAT,
    `pressure` FLOAT,
    `coordinates` BLOB,
    `measurement_time` TIME,
    `data_blob` BLOB,
    `status` VARCHAR(20),
    `readings` JSON,
    FOREIGN KEY (`robot_id`) REFERENCES `robot`(`id`)
);

-- Populate tables with test data

INSERT INTO `user` (email, nicknames, age, height, is_active, last_login) VALUES
('paul@dune.com', JSON_ARRAY('Muad''Dib', 'Usul'), 22, 180.5, 1, '2023-03-15 10:30:00'),
('leto@atreides.com', JSON_ARRAY('Duke', 'Leto the Just'), 45, 185.0, 0, '2023-02-28 15:45:00'),
('baron@harkonnen.com', JSON_ARRAY('The Baron', 'Vladimir'), 65, 170.2, 1, '2023-03-10 08:15:00'),
('chani@fremen.com', JSON_ARRAY('Sihaya'), 16, 165.5, 1, '2023-03-20 12:00:00'),
('stilgar@sietch.com', JSON_ARRAY('Naib'), 40, 178.0, 1, '2023-03-18 09:30:00'),
('jessica@bene-gesserit.com', JSON_ARRAY('Reverend Mother'), 45, 175.0, 1, '2023-03-17 14:45:00'),
('thufir@mentat.com', JSON_ARRAY('Master of Assassins'), 54, 180.0, 0, '2023-03-16 11:20:00'),
('gurney@warrior.com', NULL, 35, 183.0, 1, '2023-03-19 16:15:00'),
('duncan@swordmaster.com', JSON_ARRAY('Ghola', 'Swordmaster'), 30, 182.0, 1, '2023-03-22 08:45:00'),
('irulan@corrino.com', JSON_ARRAY('Princess', 'Historian'), 25, 170.5, 1, '2023-03-21 17:30:00'),
('feyd@harkonnen.com', NULL, 20, 175.5, 1, '2023-03-23 11:15:00'),
('liet@ecologist.com', JSON_ARRAY('Planetologist', 'Judge'), 50, 176.0, 0, '2023-03-20 14:00:00'),
('alia@child.com', JSON_ARRAY('Abomination', 'St. Alia of the Knife'), 4, 100.0, 1, '2023-03-24 09:00:00');

INSERT INTO `robot` (name, manufacture_date, specifications, serial_number) VALUES
('Hunter-Seeker', '2023-01-15', '{"speed": 50, "accuracy": 0.95, "range": 1000}', LOWER(HEX(RANDOMBLOB(16)))),
('Harvester', '2023-02-20', '{"capacity": 5000, "efficiency": 0.85, "fuel_type": "spice"}', LOWER(HEX(RANDOMBLOB(16)))),
('Ornithopter', '2023-03-01', '{"max_altitude": 10000, "passenger_capacity": 6, "stealth_mode": true}', LOWER(HEX(RANDOMBLOB(16)))),
('Sandworm Detector', '2023-03-10', '{"range": 100, "accuracy": 0.99, "power_source": "solar"}', LOWER(HEX(RANDOMBLOB(16)))),
('Spice Refinery', '2023-03-05', '{"capacity": 10000, "purity": 0.98, "automation_level": "high"}', LOWER(HEX(RANDOMBLOB(16)))),
('Weather Control', '2023-02-28', '{"range": 500, "precision": 0.95, "energy_consumption": "moderate"}', LOWER(HEX(RANDOMBLOB(16)))),
('Shield Wall Generator', '2023-02-25', '{"coverage": 1000, "strength": 0.97, "recharge_rate": "fast"}', LOWER(HEX(RANDOMBLOB(16)))),
('Distrans Communicator', '2023-03-15', '{"range": 1000, "encryption": "high", "battery_life": "1 week"}', LOWER(HEX(RANDOMBLOB(16))));

INSERT INTO `part` (name, cost, weight, dimensions, previous_prices, description) VALUES
('Suspensor', 1000.00, 5.75, X'00000000000000140000001400000000', JSON_ARRAY(950.00, 975.00, 990.00), 'Anti-gravity device'),
('Holtzman Shield', 5000.00, 2.30, X'000000000000000500000005000000', JSON_ARRAY(4800.00, 4900.00), 'Personal defense shield'),
('Spice Sensor', 2500.00, 1.20, X'000000000000000300000004000000', JSON_ARRAY(2400.00, 2450.00), 'Detects trace amounts of spice'),
('Thumper', 500.00, 10.5, X'0000000000000014000000000A000000', JSON_ARRAY(480.00, 490.00, 495.00), 'Rhythmic sand pounder'),
('Fremkit', 200.00, 2.0, X'0000000000000014000000000F000000', JSON_ARRAY(180.00, 190.00), 'Fremen desert survival kit'),
('Stillsuit', 1500.00, 3.5, X'000000000000000500000005000000', JSON_ARRAY(1400.00, 1450.00), 'Water-recycling body suit'),
('Crysknife', 3000.00, 0.5, X'000000000000000200000014000000', JSON_ARRAY(2900.00, 2950.00), 'Weapon made from sandworm tooth');

INSERT INTO `robot_parts` (robot_id, part_id, part_quantity, installation_date) VALUES
(1, 1, 2, '2023-01-20'),
(1, 2, 1, '2023-01-21'),
(2, 1, 4, '2023-02-25'),
(2, 3, 2, '2023-02-26'),
(3, 1, 6, '2023-03-05'),
(3, 2, 1, '2023-03-06'),
(3, 3, 1, '2023-03-07'),
(4, 4, 2, '2023-03-11'),
(4, 5, 1, '2023-03-12'),
(5, 1, 3, '2023-03-06'),
(5, 3, 1, '2023-03-07'),
(6, 2, 4, '2023-03-01'),
(6, 5, 2, '2023-03-02'),
(7, 1, 1, '2023-02-26'),
(7, 4, 3, '2023-02-27'),
(8, 2, 2, '2023-03-16'),
(8, 3, 1, '2023-03-17');

INSERT INTO `sensor_data` (robot_id, temperature, humidity, pressure, coordinates, measurement_time, data_blob, status, readings) VALUES
(1, 32.5, 0.15, 1013.25, X'00000000000000F7D0FF2D7CD17E00', '14:30:00', X'DEADBEEF', 'NORMAL', JSON_ARRAY(1024, 2048, 4096)),
(2, 45.2, 0.08, 1015.10, X'0000000000000058D7FF74000000', '09:15:00', X'CAFEBABE', 'WARNING', JSON_ARRAY(512, 1024, 2048)),
(3, 28.7, 0.22, 1010.50, X'00000000000000C333FF4000301E00', '18:45:00', X'FACEFEED', 'CRITICAL', JSON_ARRAY(2048, 4096, 8192)),
(4, 38.2, 0.05, 1012.75, X'000000000000009524FF1E110100', '10:45:00', X'BEEFCAFE', 'NORMAL', JSON_ARRAY(2048, 4096, 8192)),
(5, 42.8, 0.03, 1014.30, X'00000000000000924EFF1E240200', '13:20:00', X'DEADC0DE', 'WARNING', JSON_ARRAY(1024, 2048, 4096)),
(6, 36.5, 0.07, 1013.80, X'0000000000000095CBFF1E110100', '16:00:00', X'FEEDBEEF', 'NORMAL', JSON_ARRAY(512, 1024, 2048)),
(7, 39.7, 0.04, 1011.90, X'00000000000000944DFF1E2D0400', '19:30:00', X'CAFEBABE', 'CRITICAL', JSON_ARRAY(4096, 8192, 16384)),
(8, 37.1, 0.06, 1012.50, X'000000000000009524FF1E100100', '22:15:00', X'FACEFEED', 'WARNING', JSON_ARRAY(2048, 4096, 8192));

-- Views for testing
CREATE VIEW `active_users` AS
SELECT `id`, `email`, `age`, `last_login`
FROM `user`
WHERE `is_active` = 1;

CREATE VIEW `robot_part_counts` AS
SELECT r.`id` AS `robot_id`,
       r.`name` AS `robot_name`,
       COUNT(rp.`id`) AS `part_count`
FROM `robot` r
LEFT JOIN `robot_parts` rp ON rp.`robot_id` = r.`id`
GROUP BY r.`id`, r.`name`;
