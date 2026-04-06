CONNECT TO testdb@

CREATE OR REPLACE PROCEDURE get_employees_by_dept(IN p_dept_id INTEGER)
  LANGUAGE SQL
  DYNAMIC RESULT SETS 1
BEGIN
  DECLARE c1 CURSOR WITH RETURN FOR
    SELECT id, name, salary, hired FROM employees WHERE dept_id = p_dept_id;
  OPEN c1;
END@

CREATE OR REPLACE PROCEDURE get_employee_count(IN p_dept_id INTEGER, OUT p_count INTEGER)
  LANGUAGE SQL
BEGIN
  SELECT COUNT(*) INTO p_count FROM employees WHERE dept_id = p_dept_id;
END@

CREATE OR REPLACE PROCEDURE transfer_funds(
  IN p_from INTEGER,
  IN p_to INTEGER,
  IN p_amount DECIMAL(15,2)
)
  LANGUAGE SQL
BEGIN
  UPDATE accounts SET balance = balance - p_amount WHERE id = p_from;
  UPDATE accounts SET balance = balance + p_amount WHERE id = p_to;
END@

COMMIT@
