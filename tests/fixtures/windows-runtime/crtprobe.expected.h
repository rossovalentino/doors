/* Generated from Windows 11 26H2's output of `crtprobe.exe raw` on the owner's PC
 * (one string per compared step). */
    /* printf */
    "ints [42] [  -42] [7    ] [-0007] [+3] [ff] [0XFF] [10] [-5] [18446744073709551615] [4464]\r\n"
    "floats [3.141590] [2.00] [1.234568e+04] [1.230E-04] [100000] [1e-05] [1.5E-10] [0] [2] [0x1.0000000000000p+0]\r\n"
    "strings [hi] [      hi] [hi      ] [hel] [Z] [%]\r\n"
    "special [inf] [-inf] [-nan(ind)]\r\n"
    "snprintf 16 [truncat]\r\n"
    "sprintf_s 3 [7-x]\r\n"
    "_snprintf_s -1 [abcde]\r\n"
    "swprintf 10 119 53\r\n"
    "_scprintf 10\r\n"
    ,
    /* scan */
    "sscanf 7 -12 255 hello 2.5 1000.0 abc 99999999999\r\n"
    "sscanf mismatch 0\r\n"
    "sscanf empty -1\r\n"
    "sscanf_s 2 word 7\r\n"
    ,
    /* convert */
    "strtol -31 493 1295\r\n"
    "end 7\r\n"
    "overflow 2147483647 0\r\n"
    "strtoul 4294967295 18446744073709551615\r\n"
    "strtod 3.250000 -1000.000000 3 9.99989e-321\r\n"
    "atoi 123 -77 9876543210 2.75\r\n"
    "_itoa_s ffffff01\r\n"
    "_ui64toa_s 18446744073709551615\r\n"
    ,
    /* library */
    "qsort 1 2 3 4 5 6 7 8 9 10 11 12 \r\n"
    "bsearch 8\r\n"
    "strtok_s abc\r\n"
    "ctype 1 1 1 81 1\r\n"
    "cmp 1 0 0 0\r\n"
    "math 0.841471 0.540302 2.718282 2.302585 1.414214 0.463648\r\n"
    "math2 -2.0 -1.0 3.0 -2.0 1.500000 3.000000\r\n"
    "div 3 2 abs 9\r\n"
    "rand 18467 41\r\n"
    "errno text [No such file or directory]\r\n"
    ,
    /* environment */
    "getenv [door 26]\r\n"
    "getenv case [found]\r\n"
    "_dupenv_s [door 26] 8\r\n"
    "removed [gone]\r\n"
    "locale [C]\r\n"
    ,
    /* time */
    "gmtime 123 10 14 2 [Tue Nov 14 2023 22:13:20 318 PM]\r\n"
    "mkgmtime 1700000000\r\n"
    "time sane 1\r\n"
    ,
