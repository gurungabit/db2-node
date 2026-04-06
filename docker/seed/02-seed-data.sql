CONNECT TO testdb;

INSERT INTO departments (id, name) VALUES (1, 'Engineering');
INSERT INTO departments (id, name) VALUES (2, 'Sales');
INSERT INTO departments (id, name) VALUES (3, 'Marketing');

INSERT INTO employees (name, dept_id, salary, hired, active)
  VALUES ('Alice', 1, 120000.00, '2020-01-15', TRUE);
INSERT INTO employees (name, dept_id, salary, hired, active)
  VALUES ('Bob', 1, 110000.00, '2021-03-01', TRUE);
INSERT INTO employees (name, dept_id, salary, hired, active)
  VALUES ('Carol', 2, 95000.00, '2019-06-10', TRUE);
INSERT INTO employees (name, dept_id, salary, hired, active)
  VALUES ('Dave', 2, 88000.00, '2022-09-20', FALSE);
INSERT INTO employees (name, dept_id, salary, hired, active)
  VALUES ('Eve', 3, 102000.00, '2018-11-05', TRUE);

INSERT INTO accounts (id, name, balance) VALUES (1, 'Checking', 10000.00);
INSERT INTO accounts (id, name, balance) VALUES (2, 'Savings', 50000.00);
INSERT INTO accounts (id, name, balance) VALUES (3, 'Investment', 250000.00);

INSERT INTO test_nulls (col1, col2, col3, col4, col5, col6, col7, col8, col9, col10)
  VALUES (42, 'hello', 3.14, '2024-01-01', '2024-01-01-12.00.00', TRUE, 1, 100, 99.99, 'abc');
INSERT INTO test_nulls (col1, col2, col3, col4, col5, col6, col7, col8, col9, col10)
  VALUES (NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL);
INSERT INTO test_nulls (col1, col2, col3, col4, col5, col6, col7, col8, col9, col10)
  VALUES (1, NULL, 2.0, NULL, NULL, TRUE, NULL, 5, NULL, 'x');
INSERT INTO test_nulls (col1, col2, col3, col4, col5, col6, col7, col8, col9, col10)
  VALUES (NULL, 'world', NULL, '2025-12-31', NULL, NULL, 7, NULL, 100.01, NULL);

INSERT INTO test_strings (empty, ascii, unicode, long_text)
  VALUES ('', 'Hello, World!', 'こんにちは世界 🌍', CAST('' AS CLOB));
INSERT INTO test_strings (empty, ascii, unicode, long_text)
  VALUES (NULL, 'Special: "quotes'' & <brackets>', 'Ñoño café résumé naïve', NULL);
INSERT INTO test_strings (empty, ascii, unicode, long_text)
  VALUES ('', 'line1' || CHR(10) || 'line2', '中文测试数据', NULL);

INSERT INTO test_large (val, label)
  WITH t(i) AS (
    VALUES 1
    UNION ALL
    SELECT i + 1 FROM t WHERE i < 10000
  )
  SELECT i, 'Row number ' || CAST(i AS VARCHAR(10)) FROM t;

COMMIT;
