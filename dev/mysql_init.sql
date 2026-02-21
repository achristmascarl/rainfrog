-- transformed from the pg_init.sql data to mysql by claude 3 haiku
CREATE TABLE `user` (
    `id` INT AUTO_INCREMENT PRIMARY KEY,
    `email` VARCHAR(255),
    `nicknames` JSON,
    `age` SMALLINT,
    `height` FLOAT,
    `is_active` BOOLEAN,
    `last_login` TIMESTAMP,
    `_created` TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE `robot` (
    `id` INT AUTO_INCREMENT PRIMARY KEY,
    `name` VARCHAR(100),
    `manufacture_date` DATE,
    `specifications` JSON,
    `serial_number` CHAR(36),
    `_created` TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    `owner_id` INT NULL,
    FOREIGN KEY (`owner_id`) REFERENCES `user`(`id`) ON DELETE SET NULL
);

CREATE TABLE `part` (
    `id` INT AUTO_INCREMENT PRIMARY KEY,
    `name` VARCHAR(100),
    `cost` DECIMAL(10,2),
    `weight` DOUBLE,
    `dimensions` POINT,
    `previous_prices` JSON,
    `description` TEXT
);

CREATE TABLE `robot_parts` (
    `id` INT AUTO_INCREMENT PRIMARY KEY,
    `robot_id` INT NULL,
    `part_id` INT NULL,
    `part_quantity` INT,
    `installation_date` DATE,
    FOREIGN KEY (`robot_id`) REFERENCES `robot`(`id`) ON DELETE SET NULL,
    FOREIGN KEY (`part_id`) REFERENCES `part`(`id`) ON DELETE SET NULL
);

CREATE TABLE `sensor_data` (
    `id` INT AUTO_INCREMENT PRIMARY KEY,
    `robot_id` INT,
    `temperature` FLOAT,
    `humidity` FLOAT,
    `pressure` FLOAT,
    `coordinates` POINT,
    `measurement_time` TIME,
    `data_blob` BLOB,
    `status` VARCHAR(20),
    `readings` JSON,
    FOREIGN KEY (`robot_id`) REFERENCES `robot`(`id`)
);

-- Populate tables with test data

INSERT INTO `user` (email, nicknames, age, height, is_active, last_login) VALUES
('paul@dune.com', JSON_ARRAY('Muad''Dib', 'Usul'), 22, 180.5, true, '2023-03-15 10:30:00'),
('leto@atreides.com', JSON_ARRAY('Duke', 'Leto the Just'), 45, 185.0, false, '2023-02-28 15:45:00'),
('baron@harkonnen.com', JSON_ARRAY('The Baron', 'Vladimir'), 65, 170.2, true, '2023-03-10 08:15:00'),
('chani@fremen.com', JSON_ARRAY('Sihaya'), 16, 165.5, true, '2023-03-20 12:00:00'),
('stilgar@sietch.com', JSON_ARRAY('Naib'), 40, 178.0, true, '2023-03-18 09:30:00'),
('jessica@bene-gesserit.com', JSON_ARRAY('Reverend Mother'), 45, 175.0, true, '2023-03-17 14:45:00'),
('thufir@mentat.com', JSON_ARRAY('Master of Assassins'), 54, 180.0, false, '2023-03-16 11:20:00'),
('gurney@warrior.com', NULL, 35, 183.0, true, '2023-03-19 16:15:00'),
('duncan@swordmaster.com', JSON_ARRAY('Ghola', 'Swordmaster'), 30, 182.0, true, '2023-03-22 08:45:00'),
('irulan@corrino.com', JSON_ARRAY('Princess', 'Historian'), 25, 170.5, true, '2023-03-21 17:30:00'),
('feyd@harkonnen.com', NULL, 20, 175.5, true, '2023-03-23 11:15:00'),
('liet@ecologist.com', JSON_ARRAY('Planetologist', 'Judge'), 50, 176.0, false, '2023-03-20 14:00:00'),
('alia@child.com', JSON_ARRAY('Abomination', 'St. Alia of the Knife'), 4, 100.0, true, '2023-03-24 09:00:00');

INSERT INTO `robot` (name, manufacture_date, specifications, serial_number) VALUES
('Hunter-Seeker', '2023-01-15', '{"speed": 50, "accuracy": 0.95, "range": 1000}', UUID()),
('Harvester', '2023-02-20', '{"capacity": 5000, "efficiency": 0.85, "fuel_type": "spice"}', UUID()),
('Ornithopter', '2023-03-01', '{"max_altitude": 10000, "passenger_capacity": 6, "stealth_mode": true}', UUID()),
('Sandworm Detector', '2023-03-10', '{"range": 100, "accuracy": 0.99, "power_source": "solar"}', UUID()),
('Spice Refinery', '2023-03-05', '{"capacity": 10000, "purity": 0.98, "automation_level": "high"}', UUID()),
('Weather Control', '2023-02-28', '{"range": 500, "precision": 0.95, "energy_consumption": "moderate"}', UUID()),
('Shield Wall Generator', '2023-02-25', '{"coverage": 1000, "strength": 0.97, "recharge_rate": "fast"}', UUID()),
('Distrans Communicator', '2023-03-15', '{"range": 1000, "encryption": "high", "battery_life": "1 week"}', UUID());

INSERT INTO `part` (name, cost, weight, dimensions, previous_prices, description) VALUES
('Suspensor', 1000.00, 5.75, ST_POINTFROMTEXT('POINT(10 20)'), JSON_ARRAY(950.00, 975.00, 990.00), 'Anti-gravity device'),
('Holtzman Shield', 5000.00, 2.30, ST_POINTFROMTEXT('POINT(5 5)'), JSON_ARRAY(4800.00, 4900.00), 'Personal defense shield'),
('Spice Sensor', 2500.00, 1.20, ST_POINTFROMTEXT('POINT(3 4)'), JSON_ARRAY(2400.00, 2450.00), 'Detects trace amounts of spice'),
('Thumper', 500.00, 10.5, ST_POINTFROMTEXT('POINT(30 10)'), JSON_ARRAY(480.00, 490.00, 495.00), 'Rhythmic sand pounder'),
('Fremkit', 200.00, 2.0, ST_POINTFROMTEXT('POINT(20 15)'), JSON_ARRAY(180.00, 190.00), 'Fremen desert survival kit'),
('Stillsuit', 1500.00, 3.5, ST_POINTFROMTEXT('POINT(5 5)'), JSON_ARRAY(1400.00, 1450.00), 'Water-recycling body suit'),
('Crysknife', 3000.00, 0.5, ST_POINTFROMTEXT('POINT(2 20)'), JSON_ARRAY(2900.00, 2950.00), 'Weapon made from sandworm tooth'),
('Ornithopter Wing', 4000.00, 100.0, ST_POINTFROMTEXT('POINT(200 50)'), JSON_ARRAY(3800.00, 3900.00), 'Flexible aircraft wing');

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
(1, 32.5, 0.15, 1013.25, ST_POINTFROMTEXT('POINT(45.6 -122.7)'), '14:30:00', UNHEX('DEADBEEF'), 'NORMAL', JSON_ARRAY(1024, 2048, 4096)),
(2, 45.2, 0.08, 1015.10, ST_POINTFROMTEXT('POINT(40.7 -74.0)'), '09:15:00', UNHEX('CAFEBABE'), 'WARNING', JSON_ARRAY(512, 1024, 2048)),
(3, 28.7, 0.22, 1010.50, ST_POINTFROMTEXT('POINT(51.5 -0.1)'), '18:45:00', UNHEX('FACEFEED'), 'CRITICAL', JSON_ARRAY(2048, 4096, 8192)),
(4, 38.2, 0.05, 1012.75, ST_POINTFROMTEXT('POINT(35.6 -115.1)'), '10:45:00', UNHEX('BEEFCAFE'), 'NORMAL', JSON_ARRAY(2048, 4096, 8192)),
(5, 42.8, 0.03, 1014.30, ST_POINTFROMTEXT('POINT(36.1 -115.2)'), '13:20:00', UNHEX('DEADC0DE'), 'WARNING', JSON_ARRAY(1024, 2048, 4096)),
(6, 36.5, 0.07, 1013.80, ST_POINTFROMTEXT('POINT(35.9 -115.0)'), '16:00:00', UNHEX('FEEDBEEF'), 'NORMAL', JSON_ARRAY(512, 1024, 2048)),
(7, 39.7, 0.04, 1011.90, ST_POINTFROMTEXT('POINT(36.0 -115.3)'), '19:30:00', UNHEX('CAFEBABE'), 'CRITICAL', JSON_ARRAY(4096, 8192, 16384)),
(8, 37.1, 0.06, 1012.50, ST_POINTFROMTEXT('POINT(35.8 -115.1)'), '22:15:00', UNHEX('FACEFEED'), 'WARNING', JSON_ARRAY(2048, 4096, 8192));

-- Views for testing
CREATE VIEW `active_users` AS
SELECT `id`, `email`, `age`, `last_login`
FROM `user`
WHERE `is_active` = true;

CREATE VIEW `robot_part_counts` AS
SELECT r.`id` AS `robot_id`,
       r.`name` AS `robot_name`,
       COUNT(rp.`id`) AS `part_count`
FROM `robot` r
LEFT JOIN `robot_parts` rp ON rp.`robot_id` = r.`id`
GROUP BY r.`id`, r.`name`;

-- Functions for testing
CREATE FUNCTION `user_email_domain`(`email` VARCHAR(255))
RETURNS VARCHAR(255)
DETERMINISTIC
RETURN SUBSTRING_INDEX(`email`, '@', -1);

CREATE FUNCTION `robot_total_part_cost`(`target_robot_id` INT)
RETURNS DECIMAL(12,2)
READS SQL DATA
RETURN (
  SELECT COALESCE(SUM(p.`cost` * rp.`part_quantity`), 0)
  FROM `robot_parts` rp
  JOIN `part` p ON p.`id` = rp.`part_id`
  WHERE rp.`robot_id` = `target_robot_id`
);

-- Additional schema for testing
CREATE DATABASE `etl`;

USE `etl`;

CREATE TABLE `notion` (
	`id` INT NOT NULL PRIMARY KEY,
	`data` VARCHAR(255),
	`_created` TIMESTAMP
);

CREATE TABLE `airtable` (
	`id` INT NOT NULL PRIMARY KEY,
	`data` VARCHAR(255),
	`_created` TIMESTAMP
);

CREATE TABLE `slack` (
	`id` INT NOT NULL PRIMARY KEY,
	`data` VARCHAR(255),
	`_created` TIMESTAMP
);

CREATE TABLE `linear` (
	`id` INT NOT NULL PRIMARY KEY,
	`data` VARCHAR(255),
	`_created` TIMESTAMP
);

CREATE TABLE `salesforce` (
	`id` INT NOT NULL PRIMARY KEY,
	`data` VARCHAR(255),
	`_created` TIMESTAMP
);

CREATE TABLE `hubspot` (
	`id` INT NOT NULL PRIMARY KEY,
	`data` VARCHAR(255),
	`_created` TIMESTAMP
);

CREATE TABLE `trello` (
	`id` INT NOT NULL PRIMARY KEY,
	`data` VARCHAR(255),
	`_created` TIMESTAMP
);

CREATE TABLE `asana` (
	`id` INT NOT NULL PRIMARY KEY,
	`data` VARCHAR(255),
	`_created` TIMESTAMP
);

CREATE TABLE `jira` (
	`id` INT NOT NULL PRIMARY KEY,
	`data` VARCHAR(255),
	`_created` TIMESTAMP
);

CREATE TABLE `teams` (
	`id` INT NOT NULL PRIMARY KEY,
	`data` VARCHAR(255),
	`_created` TIMESTAMP
);

CREATE TABLE `gdrive` (
	`id` INT NOT NULL PRIMARY KEY,
	`data` VARCHAR(255),
	`_created` TIMESTAMP
);

CREATE TABLE `outlook` (
	`id` INT NOT NULL PRIMARY KEY,
	`data` VARCHAR(255),
	`_created` TIMESTAMP
);

CREATE TABLE `redis` (
	`id` INT NOT NULL PRIMARY KEY,
	`data` VARCHAR(255),
	`_created` TIMESTAMP
);

CREATE TABLE `github` (
	`id` INT NOT NULL PRIMARY KEY,
	`data` VARCHAR(255),
	`_created` TIMESTAMP
);

CREATE TABLE `calendar` (
	`id` INT NOT NULL PRIMARY KEY,
	`data` VARCHAR(255),
	`_created` TIMESTAMP
);

CREATE FUNCTION `etl_table_count`()
RETURNS INT
READS SQL DATA
RETURN (
  SELECT COUNT(*)
  FROM information_schema.tables
  WHERE table_schema = DATABASE()
);
