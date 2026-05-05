// DDM/DRDA Code Point Constants
// Reference: Apache Derby CodePoint.java, IBM DRDA specification

// ============================================================
// Connection / Handshake Commands
// ============================================================
pub const EXCSAT: u16 = 0x1041; // Exchange Server Attributes
pub const EXSATRD: u16 = 0x1443; // Exchange Server Attributes Reply Data
pub const ACCSEC: u16 = 0x106D; // Access Security
pub const ACCSECRD: u16 = 0x14AC; // Access Security Reply Data
pub const SECCHK: u16 = 0x106E; // Security Check
pub const SECCHKRM: u16 = 0x1219; // Security Check Reply Message
pub const ACCRDB: u16 = 0x2001; // Access RDB
pub const ACCRDBRM: u16 = 0x2201; // Access RDB Reply Message

// ============================================================
// Parameters
// ============================================================
pub const EXTNAM: u16 = 0x115E; // External Name
pub const SRVNAM: u16 = 0x116D; // Server Name
pub const SRVRLSLV: u16 = 0x115A; // Server Product Release Level
pub const SRVCLSNM: u16 = 0x1147; // Server Class Name
pub const MGRLVLLS: u16 = 0x1404; // Manager Level List
pub const CODPNT: u16 = 0x000C; // Code Point
pub const SECMEC: u16 = 0x11A2; // Security Mechanism
pub const SECTKN: u16 = 0x11DC; // Security Token
pub const ENCALG: u16 = 0x1909; // Encryption Algorithm
pub const ENCKEYLEN: u16 = 0x190A; // Encryption Key Length
pub const USRID: u16 = 0x11A0; // User ID
pub const PASSWORD: u16 = 0x11A1; // Password
pub const RDBNAM: u16 = 0x2110; // Relational Database Name
pub const PRDID: u16 = 0x112E; // Product Specific Identifier
pub const TYPDEFNAM: u16 = 0x002F; // Data Type Definition Name
pub const TYPDEFOVR: u16 = 0x0035; // TYPDEF Overrides
pub const CCSIDSBC: u16 = 0x119C; // CCSID for Single-Byte Characters
pub const CCSIDDBC: u16 = 0x119D; // CCSID for Double-Byte Characters
pub const CCSIDMBC: u16 = 0x119E; // CCSID for Mixed-Byte Characters
pub const RDBACCCL: u16 = 0x210F; // RDB Access Manager Class
pub const CRRTKN: u16 = 0x2135; // Correlation Token

// ============================================================
// SQL Operation Commands
// ============================================================
pub const CLSQRY: u16 = 0x2005; // Close Query
pub const EXCSQLIMM: u16 = 0x200A; // Execute Immediate SQL
pub const EXCSQLSTT: u16 = 0x200B; // Execute SQL Statement
pub const EXCSQLSET: u16 = 0x2014; // Execute SQL SET Statement
pub const DRPPKG: u16 = 0x2007; // Drop Package
pub const DSCSQLSTT: u16 = 0x2008; // Describe SQL Statement
pub const OPNQRY: u16 = 0x200C; // Open Query
pub const PRPSQLSTT: u16 = 0x200D; // Prepare SQL Statement
pub const RDBCMM: u16 = 0x200E; // RDB Commit Unit of Work
pub const CNTQRY: u16 = 0x2006; // Continue Query
pub const RDBRLLBCK: u16 = 0x200F; // RDB Rollback Unit of Work

// ============================================================
// SQL Reply Data Objects
// ============================================================
pub const SQLCARD: u16 = 0x2408; // SQL Communications Area Reply Data
pub const SQLDARD: u16 = 0x2411; // SQL Descriptor Area Reply Data
pub const SQLDTA: u16 = 0x2412; // SQL Program Variable Data
pub const SQLATTR: u16 = 0x2450; // SQL Statement Attributes
pub const SQLSTT: u16 = 0x2414; // SQL Statement
pub const QRYDSC: u16 = 0x241A; // Query Answer Set Description
pub const QRYDTA: u16 = 0x241B; // Query Answer Set Data
pub const EXTDTA: u16 = 0x146C; // Externalized Data, used for LOB payloads

// ============================================================
// Reply Messages
// ============================================================
pub const SVRCOD: u16 = 0x1149; // Severity Code
pub const ENDQRYRM: u16 = 0x220B; // End of Query Reply Message
pub const QRYNOPRM: u16 = 0x2202; // Query Not Open Reply Message
pub const OPNQRYRM: u16 = 0x2205; // Open Query Complete Reply Message
pub const DTAMCHRM: u16 = 0x220E; // Data Descriptor Mismatch Reply Message
pub const RDBUPDRM: u16 = 0x2218; // RDB Update Reply Message
pub const SYNTAXRM: u16 = 0x124C; // Data Stream Syntax Error Reply Message
pub const PRCCNVRM: u16 = 0x1245; // Conversational Protocol Error Reply Message
pub const CMDNSPRM: u16 = 0x1250; // Command Not Supported Reply Message
pub const PRMNSPRM: u16 = 0x1251; // Parameter Not Supported Reply Message
pub const VALNSPRM: u16 = 0x1252; // Parameter Value Not Supported Reply Message

// ============================================================
// Manager Code Points
// ============================================================
pub const AGENT: u16 = 0x1403; // Agent Manager
pub const SQLAM: u16 = 0x2407; // SQL Application Manager
pub const RDB: u16 = 0x240F; // Relational Database
pub const SECMGR: u16 = 0x1440; // Security Manager
pub const CMNTCPIP: u16 = 0x1474; // TCP/IP Communication Manager
pub const UNICODEMGR: u16 = 0x1C08; // Unicode Manager
pub const XAMGR: u16 = 0x14CC; // XA Transaction Manager

// ============================================================
// Security Mechanism Values
// ============================================================
pub const SECMEC_USRIDPWD: u16 = 0x0003; // User ID and password
pub const SECMEC_USRIDONL: u16 = 0x0004; // User ID only
pub const SECMEC_USRIDNWPWD: u16 = 0x0005; // User ID and new password
pub const SECMEC_USRENCPWD: u16 = 0x0007; // User ID and encrypted password
pub const SECMEC_EUSRIDPWD: u16 = 0x0009; // Encrypted user ID and password

// ============================================================
// Encryption Algorithm / Key Length Values
// ============================================================
pub const ENCALG_DES: u16 = 0x0001; // Data Encryption Standard
pub const ENCALG_AES: u16 = 0x0002; // Advanced Encryption Standard
pub const ENCKEYLEN_DES_56: u16 = 0x0001; // 56-bit encryption key
pub const ENCKEYLEN_AES_256: u16 = 0x0002; // 256-bit encryption key

// ============================================================
// Package / Query Parameters
// ============================================================
pub const PKGNAMCSN: u16 = 0x2113; // Package Name, Consistency Token, Section Number
pub const PKGID: u16 = 0x2159; // Package Identifier
pub const RDBCOLID: u16 = 0x2108; // RDB Collection Identifier
pub const OUTOVR: u16 = 0x2415; // Output Override
pub const OUTEXP: u16 = 0x2111; // Output Expected
pub const QRYBLKSZ: u16 = 0x2114; // Query Block Size
pub const MAXBLKEXT: u16 = 0x2141; // Maximum Number of Extra Blocks
pub const QRYPRCTYP: u16 = 0x2102; // Query Protocol Type
pub const NBRROW: u16 = 0x213A; // Number of Fetch or Insert Rows
pub const RTNSQLDA: u16 = 0x2116; // Return SQL Descriptor Area
pub const TYPSQLDA: u16 = 0x2146; // Type of SQL Descriptor Area
pub const UOWDSP: u16 = 0x210E; // Unit of Work Disposition
pub const STTDEC: u16 = 0x2101; // Statement Decimal Delimiter
pub const STTSTRDEL: u16 = 0x2104; // Statement String Delimiter
pub const PKGSNLST: u16 = 0x2139; // List of PKGNAMCSN entries
pub const PKGSN: u16 = PKGSNLST; // Legacy alias; section numbers are embedded inside PKGNAMCSN
pub const MONITOR: u16 = 0x1900; // Monitor
pub const RDBCMTOK: u16 = 0x2105; // RDB Commit Allowed
pub const QRYINSID: u16 = 0x215B; // Query Instance Identifier
pub const QRYROWSET: u16 = 0x2156; // Query Rowset Size
pub const RTNEXTDTA: u16 = 0x2148; // Return Next Data
pub const RTNEXTALL: u8 = 0x02; // Return row data and all externalized data

// ============================================================
// FD:OCA
// ============================================================
pub const FDODSC: u16 = 0x2101; // FD:OCA Data Descriptor
pub const FDODTA: u16 = 0x147A; // FD:OCA Data

// ============================================================
// Security Check Code
// ============================================================
pub const SECCHKCD: u16 = 0x11A4; // Security Check Code

// ============================================================
// Severity Code Values
// ============================================================
pub const SRVCOD_INFO: u16 = 0x0000;
pub const SRVCOD_WARNING: u16 = 0x0004;
pub const SRVCOD_ERROR: u16 = 0x0008;
pub const SRVCOD_SEVERE: u16 = 0x0010;
pub const SRVCOD_ACCDMG: u16 = 0x0014;
pub const SRVCOD_PRMDMG: u16 = 0x0018;
pub const SRVCOD_SESDMG: u16 = 0x001C;
pub const SRVCOD_SESRM: u16 = 0x0020;

// ============================================================
// Query Protocol Type Values
// ============================================================
pub const QRYPRCTYP_FIXROWPRC: u16 = 0x0002; // Fixed Row Query Protocol
pub const QRYPRCTYP_LMTBLKPRC: u16 = 0x0003; // Limited Block Query Protocol

// ============================================================
// TYPSQLDA Values
// ============================================================
pub const TYPSQLDA_STD_OUTPUT: u16 = 0x0000; // Standard output SQLDA
pub const TYPSQLDA_STD_INPUT: u16 = 0x0001; // Standard input SQLDA
pub const TYPSQLDA_LIGHT_OUTPUT: u16 = 0x0002; // Light output SQLDA
pub const TYPSQLDA_LIGHT_INPUT: u16 = 0x0003; // Light input SQLDA
pub const TYPSQLDA_X_OUTPUT: u16 = 0x0004; // Extended output SQLDA
pub const TYPSQLDA_X_INPUT: u16 = 0x0005; // Extended input SQLDA

// ============================================================
// UOWDSP Values
// ============================================================
pub const UOWDSP_COMMIT: u16 = 0x0001; // Commit at end
pub const UOWDSP_ROLLBACK: u16 = 0x0002; // Rollback at end

// ============================================================
// SQL Error Reply Messages
// ============================================================
pub const SQLERRRM: u16 = 0x2213; // SQL Error Reply Message
pub const CMDCHKRM: u16 = 0x1254; // Command Check Reply Message
pub const OBJNSPRM: u16 = 0x1253; // Object Not Supported Reply Message
pub const RDBNACRM: u16 = 0x2204; // RDB Not Accessed Reply Message
