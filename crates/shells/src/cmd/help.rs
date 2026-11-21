//! HELP and `/?` texts: the command list in cmd.exe's layout and each
//! command's syntax with its switches (worded for Doors).
use alloc::{format, string::String};

/// (name, one-line description) in HELP's order.
pub(crate) const LIST: &[(&str, &str)] = &[
    (
        "ASSOC",
        "Shows or changes which file type an extension belongs to.",
    ),
    ("ATTRIB", "Shows or changes file attributes."),
    ("BREAK", "Turns extended CTRL+C checking on or off."),
    (
        "BCDEDIT",
        "Changes the boot configuration data (administrators).",
    ),
    (
        "CACLS",
        "Shows or changes the access control lists of files.",
    ),
    ("CALL", "Runs one batch program from another and returns."),
    ("CD", "Shows or changes the current directory."),
    ("CHCP", "Shows or sets the active code page."),
    ("CHDIR", "Shows or changes the current directory."),
    (
        "CHKDSK",
        "Checks a disk and reports its status (administrators).",
    ),
    ("CHKNTFS", "Shows or changes boot-time disk checking."),
    ("CLS", "Clears the screen."),
    ("CMD", "Starts another instance of the command interpreter."),
    (
        "COLOR",
        "Sets the console's default foreground and background colors.",
    ),
    (
        "COMP",
        "Compares the contents of two files or sets of files.",
    ),
    (
        "COMPACT",
        "Shows or changes file compression on NTFS volumes.",
    ),
    (
        "CONVERT",
        "Converts FAT volumes to NTFS. The current drive can't be\n               converted.",
    ),
    ("COPY", "Copies one or more files to another location."),
    ("DATE", "Shows or sets the date."),
    ("DEL", "Deletes one or more files."),
    ("DIR", "Lists the files and subdirectories in a directory."),
    (
        "DISKPART",
        "Shows or changes disk partitions (administrators).",
    ),
    (
        "DOSKEY",
        "Edits command lines, recalls commands and creates\n               macros.",
    ),
    (
        "DRIVERQUERY",
        "Shows the installed device drivers and their state.",
    ),
    (
        "ECHO",
        "Shows messages, or turns command echoing on or off.",
    ),
    (
        "ENDLOCAL",
        "Ends the environment changes a batch file made locally.",
    ),
    ("ERASE", "Deletes one or more files."),
    ("EXIT", "Quits the command interpreter (CMD.EXE)."),
    (
        "FC",
        "Compares two files or sets of files and shows the\n               differences.",
    ),
    ("FIND", "Searches files for a text string."),
    (
        "FINDSTR",
        "Searches files for strings or regular expressions.",
    ),
    ("FOR", "Runs a command for each item in a set."),
    ("FORMAT", "Formats a disk (administrators)."),
    ("FSUTIL", "Shows or changes file system properties."),
    ("FTYPE", "Shows or changes the open commands of file types."),
    ("GOTO", "Continues a batch program at a labeled line."),
    (
        "GPRESULT",
        "Shows Group Policy information for the computer or user.",
    ),
    ("HELP", "Shows help for commands."),
    (
        "ICACLS",
        "Shows, changes, saves or restores ACLs of files and\n               directories.",
    ),
    ("IF", "Runs a command only when a condition holds."),
    (
        "LABEL",
        "Creates, changes or deletes a disk's volume label.",
    ),
    ("MD", "Creates a directory."),
    ("MKDIR", "Creates a directory."),
    ("MKLINK", "Creates symbolic links and hard links."),
    ("MODE", "Shows or configures a system device."),
    ("MORE", "Shows output one screen at a time."),
    (
        "MOVE",
        "Moves files from one directory to another, or renames\n               directories.",
    ),
    (
        "OPENFILES",
        "Shows the files remote users have open on a share.",
    ),
    ("PATH", "Shows or sets the search path for programs."),
    ("PAUSE", "Pauses a batch file until a key is pressed."),
    ("POPD", "Returns to the directory saved by PUSHD."),
    ("PRINT", "Prints a text file."),
    ("PROMPT", "Changes the command prompt."),
    ("PUSHD", "Saves the current directory, then changes it."),
    ("RD", "Removes a directory."),
    ("RECOVER", "Recovers readable data from a damaged disk."),
    ("REM", "Marks a comment in a batch file."),
    ("REN", "Renames files."),
    ("RENAME", "Renames files."),
    ("REPLACE", "Replaces files."),
    ("RMDIR", "Removes a directory."),
    ("ROBOCOPY", "Copies files and directory trees robustly."),
    ("SET", "Shows, sets or removes environment variables."),
    (
        "SETLOCAL",
        "Starts local environment changes in a batch file.",
    ),
    ("SC", "Shows or configures services."),
    ("SCHTASKS", "Schedules commands and programs."),
    (
        "SHIFT",
        "Shifts the replaceable parameters of a batch file.",
    ),
    (
        "SHUTDOWN",
        "Shuts down, restarts or signs out of the computer.",
    ),
    ("SORT", "Sorts its input."),
    ("START", "Opens a program, document or new window."),
    ("SUBST", "Associates a path with a drive letter."),
    (
        "SYSTEMINFO",
        "Shows this computer's properties and configuration.",
    ),
    ("TASKLIST", "Lists the running tasks."),
    ("TASKKILL", "Ends running processes."),
    ("TIME", "Shows or sets the time."),
    ("TITLE", "Sets the window title."),
    ("TREE", "Draws the directory structure of a drive or path."),
    ("TYPE", "Shows the contents of text files."),
    ("VER", "Shows the version."),
    ("VERIFY", "Turns write verification on or off."),
    ("VOL", "Shows a disk's volume label and serial number."),
    ("XCOPY", "Copies files and directory trees."),
    ("WMIC", "Queries management information (deprecated)."),
];

/// HELP with no argument.
pub(crate) fn list() -> String {
    let mut out =
        String::from("For more information on a specific command, type HELP command-name\n");
    for (name, text) in LIST {
        out.push_str(&format!("{name:<15}{text}\n"));
    }
    out.push_str(
        "\nFor more information on tools see the command-line reference in the online help.\n",
    );
    out
}
/// `name /?` and `HELP name`.
pub(crate) fn text(name: &str) -> String {
    let upper = name.to_ascii_uppercase();
    let body: &str = match name.to_ascii_lowercase().as_str() {
        "dir" => {
            "Lists the files and subdirectories in a directory.\n\nDIR [drive:][path][filename] [/A[[:]attributes]] [/B] [/C] [/D] [/L] [/N]\n  [/O[[:]sortorder]] [/P] [/Q] [/R] [/S] [/T[[:]timefield]] [/W] [/X] [/4]\n\n  /A    Entries with these attributes: D directories, R read-only, H hidden,\n        A ready for archiving, S system, I not content indexed, L reparse\n        points, O offline; a - prefix means not.\n  /B    Names only (no header or summary).\n  /C    Thousands separators in sizes (the default; /-C turns them off).\n  /L    Lowercase.\n  /O    Sort: N name, E extension, G directories first, S size, D date;\n        a - prefix reverses.\n  /Q    Owners.\n  /S    This directory and every subdirectory.\n  /T    Time shown: C created, A accessed, W written.\n  /W    Wide list.\n  /X    Short 8.3 names.\n\nSwitches may be preset in the DIRCMD environment variable.\n"
        }
        "copy" => {
            "Copies one or more files to another location.\n\nCOPY [/D] [/V] [/N] [/Y | /-Y] [/Z] [/L] [/A | /B ] source [/A | /B]\n     [+ source [/A | /B] [+ ...]] [destination [/A | /B]]\n\n  source       The file(s) to copy.\n  destination  The directory and/or file name for the new file(s).\n  /Y           Overwrite without asking.\n  /-Y          Ask before overwriting.\n\nJoin files with + to append them into one destination file.\n"
        }
        "move" => {
            "Moves files and renames files and directories.\n\nMOVE [/Y | /-Y] [drive:][path]filename1[,...] destination\nMOVE [/Y | /-Y] [drive:][path]dirname1 dirname2\n"
        }
        "del" | "erase" => {
            "Deletes one or more files.\n\nDEL [/P] [/F] [/S] [/Q] [/A[[:]attributes]] names\nERASE [/P] [/F] [/S] [/Q] [/A[[:]attributes]] names\n\n  /F  Delete read-only files too.\n  /S  Delete matching files in every subdirectory.\n  /Q  Don't ask before deleting with wildcards.\n  /A  Select files by attribute (R, H, S, A, I, L, O; - means not).\n"
        }
        "md" | "mkdir" => {
            "Creates a directory (and any missing parents).\n\nMKDIR [drive:]path\nMD [drive:]path\n"
        }
        "rd" | "rmdir" => {
            "Removes a directory.\n\nRMDIR [/S] [/Q] [drive:]path\nRD [/S] [/Q] [drive:]path\n\n  /S  Remove the directory tree with its files.\n  /Q  Don't ask with /S.\n"
        }
        "ren" | "rename" => {
            "Renames files.\n\nRENAME [drive:][path]filename1 filename2.\nREN [drive:][path]filename1 filename2.\n\nThe new name can't include a drive or path.\n"
        }
        "cd" | "chdir" => {
            "Shows the current directory or changes it.\n\nCHDIR [/D] [drive:][path]\nCHDIR [..]\nCD [/D] [drive:][path]\nCD [..]\n\n  ..   The parent directory.\n  /D   Change the current drive too.\n"
        }
        "echo" => {
            "Shows messages, or turns command echoing on or off.\n\n  ECHO [ON | OFF]\n  ECHO [message]\n\nType ECHO without parameters to see the current setting.\n"
        }
        "set" => {
            "Shows, sets or removes environment variables.\n\nSET [variable=[string]]\nSET /A expression\nSET /P variable=[promptString]\n\n  /A  Evaluates an integer expression: ( ) ! ~ - * / % + - << >> & ^ |\n      = *= /= %= += -= &= ^= |= <<= >>= and , between expressions.\n  /P  Sets the variable to a line of input after showing the prompt.\n\n%VAR:str1=str2% replaces text and %VAR:~start,length% takes a substring\nwhen a variable is expanded.\n"
        }
        "if" => {
            "Runs a command only when a condition holds.\n\nIF [NOT] ERRORLEVEL number command\nIF [/I] [NOT] string1==string2 command\nIF [NOT] EXIST filename command\nIF [/I] string1 compare-op string2 command\nIF [NOT] DEFINED variable command\nIF CMDEXTVERSION number command\n\n  compare-op: EQU NEQ LSS LEQ GTR GEQ (numbers compare as numbers)\n\nAn ELSE clause may follow a parenthesized command:\n  IF EXIST file (del file) ELSE (echo missing)\n"
        }
        "for" => {
            "Runs a command for each item in a set.\n\nFOR %variable IN (set) DO command [parameters]\nFOR /D %variable IN (set) DO command       directories\nFOR /R [[drive:]path] %variable IN (set) DO command   directory tree\nFOR /L %variable IN (start,step,end) DO command       numbers\nFOR /F [\"options\"] %variable IN (file-set | \"string\" | 'command') DO command\n\n  /F options: eol=c skip=n delims=xxx tokens=x,y,m-n* usebackq\n\nUse %%variable in batch files. Modifiers: %~I %~fI %~dI %~pI %~nI %~xI\n%~sI %~aI %~tI %~zI %~$PATH:I, combinable (%~dpI, %~nxI, %~ftzaI).\n"
        }
        "goto" => {
            "Continues a batch program at a labeled line.\n\nGOTO label\n\nGOTO :EOF ends the batch file (or a CALLed label).\n"
        }
        "call" => {
            "Runs one batch program from another and returns.\n\nCALL [drive:][path]filename [batch-parameters]\nCALL :label arguments\n"
        }
        "exit" => {
            "Quits CMD.EXE or the current batch file.\n\nEXIT [/B] [exitCode]\n\n  /B        Leave the batch file (or CALLed label) instead of CMD.EXE.\n  exitCode  Sets ERRORLEVEL (with /B) or the process exit code.\n"
        }
        "type" => "Shows the contents of text files.\n\nTYPE [drive:][path]filename\n",
        "start" => {
            "Opens a program, document, folder or new window.\n\nSTART [\"title\"] [/D path] [/I] [/MIN] [/MAX] [/WAIT] [/B]\n      [command/program] [parameters]\n\nDocuments open with their associated app; URLs open in the browser.\n"
        }
        "setlocal" => {
            "Starts local environment changes in a batch file.\n\nSETLOCAL [ENABLEEXTENSIONS | DISABLEEXTENSIONS]\n         [ENABLEDELAYEDEXPANSION | DISABLEDELAYEDEXPANSION]\n\nWith delayed expansion, !VAR! expands when each command runs.\n"
        }
        "endlocal" => "Ends the local environment changes of SETLOCAL.\n\nENDLOCAL\n",
        "shift" => "Shifts the replaceable parameters of a batch file.\n\nSHIFT [/n]\n",
        "pushd" => "Saves the current directory, then changes it.\n\nPUSHD [path | ..]\n",
        "popd" => "Returns to the directory PUSHD saved.\n\nPOPD\n",
        "pause" => "Pauses a batch file and shows\n    Press any key to continue . . .\n\nPAUSE\n",
        "title" => "Sets the window title.\n\nTITLE [string]\n",
        "color" => {
            "Sets the default console colors.\n\nCOLOR [attr]\n\n  attr  Two hex digits: background, then foreground:\n        0 Black 1 Blue 2 Green 3 Aqua 4 Red 5 Purple 6 Yellow 7 White\n        8 Gray 9 Light Blue A Light Green B Light Aqua C Light Red\n        D Light Purple E Light Yellow F Bright White\n"
        }
        "cls" => "Clears the screen.\n\nCLS\n",
        "ver" => "Shows the version.\n\nVER\n",
        "vol" => "Shows a disk's volume label and serial number.\n\nVOL [drive:]\n",
        "path" => {
            "Shows or sets the search path for programs.\n\nPATH [[drive:]path[;...][;%PATH%]]\nPATH ;\n\nPATH ; clears the path.\n"
        }
        "prompt" => {
            "Changes the command prompt.\n\nPROMPT [text]\n\n  $P path  $G >  $L <  $B |  $Q =  $D date  $T time  $N drive\n  $V version  $_ new line  $$ $  $A &  $C (  $F )  $S space  $+ PUSHD depth\n"
        }
        "date" => "Shows or sets the date.\n\nDATE [/T | date]\n",
        "time" => "Shows or sets the time.\n\nTIME [/T | time]\n",
        "rem" => "Marks a comment in a batch file.\n\nREM [comment]\n",
        "assoc" => "Shows or changes file extension associations.\n\nASSOC [.ext[=[fileType]]]\n",
        "ftype" => {
            "Shows or changes the open commands of file types.\n\nFTYPE [fileType[=[openCommandString]]]\n"
        }
        "mklink" => "Creates a symbolic link.\n\nMKLINK [[/D] | [/H] | [/J]] Link Target\n",
        "verify" => "Turns write verification on or off.\n\nVERIFY [ON | OFF]\n",
        "break" => "Turns extended CTRL+C checking on or off.\n\nBREAK [ON | OFF]\n",
        "find" => {
            "Searches files for a text string.\n\nFIND [/V] [/C] [/N] [/I] [/OFF[LINE]] \"string\" [[drive:][path]filename[ ...]]\n\n  /V  Lines without the string.  /C  Only the count.\n  /N  Line numbers.               /I  Ignore case.\n"
        }
        "findstr" => {
            "Searches files for strings.\n\nFINDSTR [/B] [/E] [/L] [/R] [/S] [/I] [/X] [/V] [/N] [/M] [/O] [/P]\n        [/C:string] [/G:file] [/D:dir list] strings [[drive:][path]filename[ ...]]\n\n  /B line start  /E line end  /L literal  /R regular expression (default)\n  /S subdirectories  /I ignore case  /X whole lines  /V non-matching lines\n  /N line numbers  /M file names only  /O offsets  /C:string literal string\n\nRegular expressions: . * ^ $ [class] [^class] [x-y] \\< \\> \\x\n"
        }
        "sort" => {
            "Sorts input.\n\nSORT [/R] [/+n] [[drive1:][path1]filename1] [/O [drive2:][path2]filename2]\n"
        }
        "more" => {
            "Shows output one screen at a time.\n\nMORE [drive:][path]filename\ncommand-name | MORE\n\nSpace: next page   Enter: next line   Q: quit\n"
        }
        "tree" => {
            "Draws the folder structure of a drive or path.\n\nTREE [drive:][path] [/F] [/A]\n\n  /F  Files too.   /A  ASCII characters for the lines.\n"
        }
        "attrib" => {
            "Shows or changes file attributes.\n\nATTRIB [+R | -R] [+A | -A] [+S | -S] [+H | -H] [+I | -I]\n       [drive:][path][filename] [/S [/D]]\n"
        }
        "xcopy" => {
            "Copies files and directory trees.\n\nXCOPY source [destination] [/S] [/E] [/I] [/Y] [/Q] [/F] [/L] [/H] [/T]\n\n  /S  Subdirectories (not empty ones)   /E  Subdirectories, empty ones too\n  /I  Destination is a directory        /Y  Overwrite without asking\n  /Q  Quiet  /F  Full names  /L  List only  /H  Hidden and system files\n  /T  Directory structure only\n"
        }
        "where" => {
            "Shows where files matching a pattern are.\n\nWHERE [/R dir] [/Q] [/F] [/T] pattern...\n"
        }
        "whoami" => {
            "Shows the signed-in user.\n\nWHOAMI [/UPN | /FQDN | /LOGONID | /USER | /GROUPS | /PRIV | /ALL]\n"
        }
        "hostname" => "Shows the computer's name.\n\nhostname\n",
        "ipconfig" => {
            "Shows the network configuration.\n\nipconfig [/all | /release | /renew | /flushdns | /displaydns]\n"
        }
        "ping" => {
            "Checks whether a computer answers.\n\nping [-t] [-n count] [-l size] [-w timeout] [-4] target_name\n"
        }
        "tasklist" => {
            "Lists running processes.\n\nTASKLIST [/V] [/FI filter] [/FO TABLE|LIST|CSV] [/NH]\n"
        }
        "taskkill" => {
            "Ends processes.\n\nTASKKILL [/F] [/T] { [/PID processid] | [/IM imagename] }\n"
        }
        "timeout" => {
            "Waits for a number of seconds.\n\nTIMEOUT [/T] timeout [/NOBREAK]\n\n  timeout  Seconds (-1 waits for a key).\n"
        }
        "choice" => {
            "Asks for one of a set of keys.\n\nCHOICE [/C choices] [/N] [/CS] [/T timeout /D choice] [/M text]\n\nERRORLEVEL is the position of the chosen key.\n"
        }
        "clip" => "Copies its input to the clipboard.\n\ncommand | CLIP\nCLIP < filename\n",
        "fc" => {
            "Compares two files.\n\nFC [/A] [/C] [/L] [/N] [/W] [drive1:][path1]filename1 [drive2:][path2]filename2\nFC /B [drive1:][path1]filename1 [drive2:][path2]filename2\n"
        }
        "comp" => {
            "Compares two files byte by byte.\n\nCOMP [data1] [data2] [/D] [/A] [/L] [/N=number] [/C] [/M]\n"
        }
        "systeminfo" => {
            "Shows this computer's properties.\n\nSYSTEMINFO [/FO TABLE|LIST|CSV] [/NH]\n"
        }
        "chcp" => "Shows or sets the active code page.\n\nCHCP [nnn]\n",
        "mode" => "Shows or configures a device.\n\nMODE [CON[:]] [COLS=c] [LINES=n]\n",
        "reg" => {
            "Reads and changes the registry.\n\nREG QUERY | ADD | DELETE | COPY | EXPORT | IMPORT\n\nREG QUERY KeyName [/v ValueName | /ve] [/s]\nREG ADD KeyName [/v ValueName | /ve] [/t Type] [/d Data] [/f]\nREG DELETE KeyName [/v ValueName | /ve | /va] [/f]\n"
        }
        "setx" => "Saves a user environment variable.\n\nSETX var value [/M]\n",
        "certutil" => {
            "Certificate and hash utility.\n\nCertUtil -hashfile InFile [MD5 | SHA1 | SHA256 | SHA384 | SHA512]\n"
        }
        "shutdown" => {
            "Shuts down, restarts or signs out.\n\nshutdown [/s | /r | /l | /a] [/t xxx]\n"
        }
        "cmd" => {
            "Starts a new instance of the command interpreter.\n\nCMD [/C | /K] [/Q] [/D] [/V:ON | /V:OFF] [[/S] string]\n\n  /C  Run the command, then exit.   /K  Run the command and stay.\n  /V:ON  Delayed expansion with !VAR!.\n"
        }
        "help" => "Shows help for commands.\n\nHELP [command]\n",
        _ => "",
    };
    if body.is_empty() {
        match LIST.iter().find(|(n, _)| *n == upper) {
            Some((_, d)) => format!("{d}\n\n{upper}\n"),
            None => {
                format!("This command is not supported by the help utility.  Try \"{name} /?\".\n")
            }
        }
    } else {
        String::from(body)
    }
}
