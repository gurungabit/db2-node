CONNECT TO testdb;

DROP TABLE IF EXISTS employees;
DROP TABLE IF EXISTS departments;
DROP TABLE IF EXISTS accounts;
DROP TABLE IF EXISTS test_large;
DROP TABLE IF EXISTS test_nulls;
DROP TABLE IF EXISTS test_strings;
DROP TABLE IF EXISTS test_types;

CREATE TABLE test_types (
  id            INTEGER NOT NULL GENERATED ALWAYS AS IDENTITY,
  col_smallint  SMALLINT,
  col_integer   INTEGER,
  col_bigint    BIGINT,
  col_real      REAL,
  col_double    DOUBLE,
  col_decimal   DECIMAL(15,2),
  col_numeric   NUMERIC(10,4),
  col_char      CHAR(20),
  col_varchar   VARCHAR(255),
  col_clob      CLOB(64K),
  col_binary    CHAR(16) FOR BIT DATA,
  col_varbinary VARCHAR(256) FOR BIT DATA,
  col_blob      BLOB(64K),
  col_date      DATE,
  col_time      TIME,
  col_timestamp TIMESTAMP,
  col_boolean   BOOLEAN,
  col_xml       XML,
  PRIMARY KEY (id)
);

CREATE TABLE test_strings (
  id      INTEGER NOT NULL GENERATED ALWAYS AS IDENTITY,
  empty   VARCHAR(10),
  ascii   VARCHAR(255),
  unicode VARCHAR(255),
  long_text CLOB(1M),
  PRIMARY KEY (id)
);

CREATE TABLE test_nulls (
  id    INTEGER NOT NULL GENERATED ALWAYS AS IDENTITY,
  col1  INTEGER,
  col2  VARCHAR(50),
  col3  DOUBLE,
  col4  DATE,
  col5  TIMESTAMP,
  col6  BOOLEAN,
  col7  SMALLINT,
  col8  BIGINT,
  col9  DECIMAL(10,2),
  col10 CHAR(10),
  PRIMARY KEY (id)
);

CREATE TABLE test_large (
  id    INTEGER NOT NULL GENERATED ALWAYS AS IDENTITY,
  val   INTEGER NOT NULL,
  label VARCHAR(100) NOT NULL,
  PRIMARY KEY (id)
);

CREATE TABLE accounts (
  id      INTEGER NOT NULL,
  name    VARCHAR(100) NOT NULL,
  balance DECIMAL(15,2) NOT NULL DEFAULT 0,
  PRIMARY KEY (id)
);

CREATE TABLE departments (
  id   INTEGER NOT NULL,
  name VARCHAR(100) NOT NULL,
  PRIMARY KEY (id)
);

CREATE TABLE employees (
  id      INTEGER NOT NULL GENERATED ALWAYS AS IDENTITY,
  name    VARCHAR(100) NOT NULL,
  dept_id INTEGER,
  salary  DECIMAL(10,2),
  hired   DATE,
  active  BOOLEAN DEFAULT TRUE,
  PRIMARY KEY (id),
  FOREIGN KEY (dept_id) REFERENCES departments(id)
);

COMMIT;
