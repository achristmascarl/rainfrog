CONNECT rainfrog/password@localhost/rainfrog;

CREATE SEQUENCE user_seq START WITH 1 INCREMENT BY 1;
CREATE SEQUENCE robot_seq START WITH 1 INCREMENT BY 1;
CREATE SEQUENCE part_seq START WITH 1 INCREMENT BY 1;
CREATE SEQUENCE robot_parts_seq START WITH 1 INCREMENT BY 1;
CREATE SEQUENCE sensor_data_seq START WITH 1 INCREMENT BY 1;

CREATE TABLE "user" (
    id NUMBER(10) PRIMARY KEY,
    email VARCHAR2(255),
    nicknames CLOB,
    age NUMBER(5),
    height FLOAT,
    is_active NUMBER(1),
    last_login TIMESTAMP,
    "_created" TIMESTAMP DEFAULT SYSTIMESTAMP
);

CREATE TABLE robot (
    id NUMBER(10) PRIMARY KEY,
    name VARCHAR2(100),
    manufacture_date DATE,
    specifications CLOB,
    serial_number CHAR(36),
    "_created" TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    owner_id NUMBER(10),
    CONSTRAINT fk_robot_owner FOREIGN KEY (owner_id) REFERENCES "user"(id) ON DELETE SET NULL
);

CREATE TABLE part (
    id NUMBER(10) PRIMARY KEY,
    name VARCHAR2(100),
    cost NUMBER(10,2),
    weight FLOAT,
    dimensions SDO_GEOMETRY,
    previous_prices CLOB,
    description CLOB
);

CREATE TABLE robot_parts (
    id NUMBER(10) PRIMARY KEY,
    robot_id NUMBER(10),
    part_id NUMBER(10),
    part_quantity NUMBER(10),
    installation_date DATE,
    CONSTRAINT fk_robot_parts_robot FOREIGN KEY (robot_id) REFERENCES robot(id) ON DELETE SET NULL,
    CONSTRAINT fk_robot_parts_part FOREIGN KEY (part_id) REFERENCES part(id) ON DELETE SET NULL
);

CREATE TABLE sensor_data (
    id NUMBER(10) PRIMARY KEY,
    robot_id NUMBER(10),
    temperature FLOAT,
    humidity FLOAT,
    pressure FLOAT,
    coordinates SDO_GEOMETRY,
    measurement_time TIMESTAMP,
    data_blob BLOB,
    status VARCHAR2(20),
    readings CLOB,
    CONSTRAINT fk_sensor_data_robot FOREIGN KEY (robot_id) REFERENCES robot(id)
);

CREATE OR REPLACE TRIGGER user_trg
BEFORE INSERT ON "user"
FOR EACH ROW
BEGIN
    SELECT user_seq.NEXTVAL INTO :new.id FROM dual;
END;
/

CREATE OR REPLACE TRIGGER robot_trg
BEFORE INSERT ON robot
FOR EACH ROW
BEGIN
    SELECT robot_seq.NEXTVAL INTO :new.id FROM dual;
END;
/

CREATE OR REPLACE TRIGGER part_trg
BEFORE INSERT ON part
FOR EACH ROW
BEGIN
    SELECT part_seq.NEXTVAL INTO :new.id FROM dual;
END;
/

CREATE OR REPLACE TRIGGER robot_parts_trg
BEFORE INSERT ON robot_parts
FOR EACH ROW
BEGIN
    SELECT robot_parts_seq.NEXTVAL INTO :new.id FROM dual;
END;
/

CREATE OR REPLACE TRIGGER sensor_data_trg
BEFORE INSERT ON sensor_data
FOR EACH ROW
BEGIN
    SELECT sensor_data_seq.NEXTVAL INTO :new.id FROM dual;
END;
/

-- Populate tables with test data

INSERT INTO "user" (email, nicknames, age, height, is_active, last_login) VALUES
('paul@dune.com', '["Muad''Dib", "Usul"]', 22, 180.5, 1, TIMESTAMP '2023-03-15 10:30:00');
INSERT INTO "user" (email, nicknames, age, height, is_active, last_login) VALUES
('leto@atreides.com', '["Duke", "Leto the Just"]', 45, 185.0, 0, TIMESTAMP '2023-02-28 15:45:00');
INSERT INTO "user" (email, nicknames, age, height, is_active, last_login) VALUES
('baron@harkonnen.com', '["The Baron", "Vladimir"]', 65, 170.2, 1, TIMESTAMP '2023-03-10 08:15:00');
INSERT INTO "user" (email, nicknames, age, height, is_active, last_login) VALUES
('chani@fremen.com', '["Sihaya"]', 16, 165.5, 1, TIMESTAMP '2023-03-20 12:00:00');
INSERT INTO "user" (email, nicknames, age, height, is_active, last_login) VALUES
('stilgar@sietch.com', '["Naib"]', 40, 178.0, 1, TIMESTAMP '2023-03-18 09:30:00');
INSERT INTO "user" (email, nicknames, age, height, is_active, last_login) VALUES
('jessica@bene-gesserit.com', '["Reverend Mother"]', 45, 175.0, 1, TIMESTAMP '2023-03-17 14:45:00');
INSERT INTO "user" (email, nicknames, age, height, is_active, last_login) VALUES
('thufir@mentat.com', '["Master of Assassins"]', 54, 180.0, 0, TIMESTAMP '2023-03-16 11:20:00');
INSERT INTO "user" (email, nicknames, age, height, is_active, last_login) VALUES
('gurney@warrior.com', NULL, 35, 183.0, 1, TIMESTAMP '2023-03-19 16:15:00');
INSERT INTO "user" (email, nicknames, age, height, is_active, last_login) VALUES
('duncan@swordmaster.com', '["Ghola", "Swordmaster"]', 30, 182.0, 1, TIMESTAMP '2023-03-22 08:45:00');
INSERT INTO "user" (email, nicknames, age, height, is_active, last_login) VALUES
('irulan@corrino.com', '["Princess", "Historian"]', 25, 170.5, 1, TIMESTAMP '2023-03-21 17:30:00');
INSERT INTO "user" (email, nicknames, age, height, is_active, last_login) VALUES
('feyd@harkonnen.com', NULL, 20, 175.5, 1, TIMESTAMP '2023-03-23 11:15:00');
INSERT INTO "user" (email, nicknames, age, height, is_active, last_login) VALUES
('liet@ecologist.com', '["Planetologist", "Judge"]', 50, 176.0, 0, TIMESTAMP '2023-03-20 14:00:00');
INSERT INTO "user" (email, nicknames, age, height, is_active, last_login) VALUES
('alia@child.com', '["Abomination", "St. Alia of the Knife"]', 4, 100.0, 1, TIMESTAMP '2023-03-24 09:00:00');

INSERT INTO robot (name, manufacture_date, specifications, serial_number) VALUES
('Hunter-Seeker', TO_DATE('2023-01-15', 'YYYY-MM-DD'), '{"speed": 50, "accuracy": 0.95, "range": 1000}', SYS_GUID());
INSERT INTO robot (name, manufacture_date, specifications, serial_number) VALUES
('Harvester', TO_DATE('2023-02-20', 'YYYY-MM-DD'), '{"capacity": 5000, "efficiency": 0.85, "fuel_type": "spice"}', SYS_GUID());
INSERT INTO robot (name, manufacture_date, specifications, serial_number) VALUES
('Ornithopter', TO_DATE('2023-03-01', 'YYYY-MM-DD'), '{"max_altitude": 10000, "passenger_capacity": 6, "stealth_mode": true}', SYS_GUID());
INSERT INTO robot (name, manufacture_date, specifications, serial_number) VALUES
('Sandworm Detector', TO_DATE('2023-03-10', 'YYYY-MM-DD'), '{"range": 100, "accuracy": 0.99, "power_source": "solar"}', SYS_GUID());
INSERT INTO robot (name, manufacture_date, specifications, serial_number) VALUES
('Spice Refinery', TO_DATE('2023-03-05', 'YYYY-MM-DD'), '{"capacity": 10000, "purity": 0.98, "automation_level": "high"}', SYS_GUID());
INSERT INTO robot (name, manufacture_date, specifications, serial_number) VALUES
('Weather Control', TO_DATE('2023-02-28', 'YYYY-MM-DD'), '{"range": 500, "precision": 0.95, "energy_consumption": "moderate"}', SYS_GUID());
INSERT INTO robot (name, manufacture_date, specifications, serial_number) VALUES
('Shield Wall Generator', TO_DATE('2023-02-25', 'YYYY-MM-DD'), '{"coverage": 1000, "strength": 0.97, "recharge_rate": "fast"}', SYS_GUID());
INSERT INTO robot (name, manufacture_date, specifications, serial_number) VALUES
('Distrans Communicator', TO_DATE('2023-03-15', 'YYYY-MM-DD'), '{"range": 1000, "encryption": "high", "battery_life": "1 week"}', SYS_GUID());

INSERT INTO part (name, cost, weight, dimensions, previous_prices, description) VALUES
('Suspensor', 1000.00, 5.75, SDO_GEOMETRY(2001, NULL, SDO_POINT_TYPE(10, 20, NULL), NULL, NULL), '[950.00, 975.00, 990.00]', 'Anti-gravity device');
INSERT INTO part (name, cost, weight, dimensions, previous_prices, description) VALUES
('Holtzman Shield', 5000.00, 2.30, SDO_GEOMETRY(2001, NULL, SDO_POINT_TYPE(5, 5, NULL), NULL, NULL), '[4800.00, 4900.00]', 'Personal defense shield');
INSERT INTO part (name, cost, weight, dimensions, previous_prices, description) VALUES
('Spice Sensor', 2500.00, 1.20, SDO_GEOMETRY(2001, NULL, SDO_POINT_TYPE(3, 4, NULL), NULL, NULL), '[2400.00, 2450.00]', 'Detects trace amounts of spice');
INSERT INTO part (name, cost, weight, dimensions, previous_prices, description) VALUES
('Thumper', 500.00, 10.5, SDO_GEOMETRY(2001, NULL, SDO_POINT_TYPE(30, 10, NULL), NULL, NULL), '[480.00, 490.00, 495.00]', 'Rhythmic sand pounder');
INSERT INTO part (name, cost, weight, dimensions, previous_prices, description) VALUES
('Fremkit', 200.00, 2.0, SDO_GEOMETRY(2001, NULL, SDO_POINT_TYPE(20, 15, NULL), NULL, NULL), '[180.00, 190.00]', 'Fremen desert survival kit');
INSERT INTO part (name, cost, weight, dimensions, previous_prices, description) VALUES
('Stillsuit', 1500.00, 3.5, SDO_GEOMETRY(2001, NULL, SDO_POINT_TYPE(5, 5, NULL), NULL, NULL), '[1400.00, 1450.00]', 'Water-recycling body suit');
INSERT INTO part (name, cost, weight, dimensions, previous_prices, description) VALUES
('Crysknife', 3000.00, 0.5, SDO_GEOMETRY(2001, NULL, SDO_POINT_TYPE(2, 20, NULL), NULL, NULL), '[2900.00, 2950.00]', 'Weapon made from sandworm tooth');
INSERT INTO part (name, cost, weight, dimensions, previous_prices, description) VALUES
('Ornithopter Wing', 4000.00, 100.0, SDO_GEOMETRY(2001, NULL, SDO_POINT_TYPE(200, 50, NULL), NULL, NULL), '[3800.00, 3900.00]', 'Flexible aircraft wing');

INSERT INTO robot_parts (robot_id, part_id, part_quantity, installation_date) VALUES
(1, 1, 2, TO_DATE('2023-01-20', 'YYYY-MM-DD'));
INSERT INTO robot_parts (robot_id, part_id, part_quantity, installation_date) VALUES
(1, 2, 1, TO_DATE('2023-01-21', 'YYYY-MM-DD'));
INSERT INTO robot_parts (robot_id, part_id, part_quantity, installation_date) VALUES
(2, 1, 4, TO_DATE('2023-02-25', 'YYYY-MM-DD'));
INSERT INTO robot_parts (robot_id, part_id, part_quantity, installation_date) VALUES
(2, 3, 2, TO_DATE('2023-02-26', 'YYYY-MM-DD'));
INSERT INTO robot_parts (robot_id, part_id, part_quantity, installation_date) VALUES
(3, 1, 6, TO_DATE('2023-03-05', 'YYYY-MM-DD'));
INSERT INTO robot_parts (robot_id, part_id, part_quantity, installation_date) VALUES
(3, 2, 1, TO_DATE('2023-03-06', 'YYYY-MM-DD'));
INSERT INTO robot_parts (robot_id, part_id, part_quantity, installation_date) VALUES
(3, 3, 1, TO_DATE('2023-03-07', 'YYYY-MM-DD'));
INSERT INTO robot_parts (robot_id, part_id, part_quantity, installation_date) VALUES
(4, 4, 2, TO_DATE('2023-03-11', 'YYYY-MM-DD'));
INSERT INTO robot_parts (robot_id, part_id, part_quantity, installation_date) VALUES
(4, 5, 1, TO_DATE('2023-03-12', 'YYYY-MM-DD'));
INSERT INTO robot_parts (robot_id, part_id, part_quantity, installation_date) VALUES
(5, 1, 3, TO_DATE('2023-03-06', 'YYYY-MM-DD'));
INSERT INTO robot_parts (robot_id, part_id, part_quantity, installation_date) VALUES
(5, 3, 1, TO_DATE('2023-03-07', 'YYYY-MM-DD'));
INSERT INTO robot_parts (robot_id, part_id, part_quantity, installation_date) VALUES
(6, 2, 4, TO_DATE('2023-03-01', 'YYYY-MM-DD'));
INSERT INTO robot_parts (robot_id, part_id, part_quantity, installation_date) VALUES
(6, 5, 2, TO_DATE('2023-03-02', 'YYYY-MM-DD'));
INSERT INTO robot_parts (robot_id, part_id, part_quantity, installation_date) VALUES
(7, 1, 1, TO_DATE('2023-02-26', 'YYYY-MM-DD'));
INSERT INTO robot_parts (robot_id, part_id, part_quantity, installation_date) VALUES
(7, 4, 3, TO_DATE('2023-02-27', 'YYYY-MM-DD'));
INSERT INTO robot_parts (robot_id, part_id, part_quantity, installation_date) VALUES
(8, 2, 2, TO_DATE('2023-03-16', 'YYYY-MM-DD'));
INSERT INTO robot_parts (robot_id, part_id, part_quantity, installation_date) VALUES
(8, 3, 1, TO_DATE('2023-03-17', 'YYYY-MM-DD'));

INSERT INTO sensor_data (robot_id, temperature, humidity, pressure, coordinates, measurement_time, data_blob, status, readings) VALUES
(1, 32.5, 0.15, 1013.25, SDO_GEOMETRY(2001, NULL, SDO_POINT_TYPE(45.6, -122.7, NULL), NULL, NULL),
  TO_TIMESTAMP('14:30:00', 'HH24:MI:SS'), HEXTORAW('DEADBEEF'), 'NORMAL', '[1024, 2048, 4096]');
INSERT INTO sensor_data (robot_id, temperature, humidity, pressure, coordinates, measurement_time, data_blob, status, readings) VALUES
(2, 45.2, 0.08, 1015.10, SDO_GEOMETRY(2001, NULL, SDO_POINT_TYPE(40.7, -74.0, NULL), NULL, NULL),
  TO_TIMESTAMP('09:15:00', 'HH24:MI:SS'), HEXTORAW('CAFEBABE'), 'WARNING', '[512, 1024, 2048]');
INSERT INTO sensor_data (robot_id, temperature, humidity, pressure, coordinates, measurement_time, data_blob, status, readings) VALUES
(3, 28.7, 0.22, 1010.50, SDO_GEOMETRY(2001, NULL, SDO_POINT_TYPE(51.5, -0.1, NULL), NULL, NULL),
  TO_TIMESTAMP('18:45:00', 'HH24:MI:SS'), HEXTORAW('FACEFEED'), 'CRITICAL', '[2048, 4096, 8192]');
INSERT INTO sensor_data (robot_id, temperature, humidity, pressure, coordinates, measurement_time, data_blob, status, readings) VALUES
(4, 38.2, 0.05, 1012.75, SDO_GEOMETRY(2001, NULL, SDO_POINT_TYPE(35.6, -115.1, NULL), NULL, NULL),
  TO_TIMESTAMP('10:45:00', 'HH24:MI:SS'), HEXTORAW('BEEFCAFE'), 'NORMAL', '[2048, 4096, 8192]');
INSERT INTO sensor_data (robot_id, temperature, humidity, pressure, coordinates, measurement_time, data_blob, status, readings) VALUES
(5, 42.8, 0.03, 1014.30, SDO_GEOMETRY(2001, NULL, SDO_POINT_TYPE(36.1, -115.2, NULL), NULL, NULL),
  TO_TIMESTAMP('13:20:00', 'HH24:MI:SS'), HEXTORAW('DEADC0DE'), 'WARNING', '[1024, 2048, 4096]');
INSERT INTO sensor_data (robot_id, temperature, humidity, pressure, coordinates, measurement_time, data_blob, status, readings) VALUES
(6, 36.5, 0.07, 1013.80, SDO_GEOMETRY(2001, NULL, SDO_POINT_TYPE(35.9, -115.0, NULL), NULL, NULL),
  TO_TIMESTAMP('16:00:00', 'HH24:MI:SS'), HEXTORAW('FEEDBEEF'), 'NORMAL', '[512, 1024, 2048]');
INSERT INTO sensor_data (robot_id, temperature, humidity, pressure, coordinates, measurement_time, data_blob, status, readings) VALUES
(7, 39.7, 0.04, 1011.90, SDO_GEOMETRY(2001, NULL, SDO_POINT_TYPE(36.0, -115.3, NULL), NULL, NULL),
  TO_TIMESTAMP('19:30:00', 'HH24:MI:SS'), HEXTORAW('CAFEBABE'), 'CRITICAL', '[4096, 8192, 16384]');
INSERT INTO sensor_data (robot_id, temperature, humidity, pressure, coordinates, measurement_time, data_blob, status, readings) VALUES
(8, 37.1, 0.06, 1012.50, SDO_GEOMETRY(2001, NULL, SDO_POINT_TYPE(35.8, -115.1, NULL), NULL, NULL),
  TO_TIMESTAMP('22:15:00', 'HH24:MI:SS'), HEXTORAW('FACEFEED'), 'WARNING', '[2048, 4096, 8192]');

-- Views for testing
CREATE VIEW active_users AS
SELECT id, email, age, last_login
FROM "user"
WHERE is_active = 1;

CREATE VIEW robot_part_counts AS
SELECT r.id AS robot_id,
       r.name AS robot_name,
       COUNT(rp.id) AS part_count
FROM robot r
LEFT JOIN robot_parts rp ON rp.robot_id = r.id
GROUP BY r.id, r.name;

CREATE MATERIALIZED VIEW robot_part_costs AS
SELECT r.id AS robot_id,
       r.name AS robot_name,
       COALESCE(SUM(p.cost * rp.part_quantity), 0) AS total_part_cost
FROM robot r
LEFT JOIN robot_parts rp ON rp.robot_id = r.id
LEFT JOIN part p ON p.id = rp.part_id
GROUP BY r.id, r.name;

COMMIT;
