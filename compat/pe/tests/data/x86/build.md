# PE32 loader fixtures

`x86app.exe` and `x86dll.dll` are real MSVC x86 images for `tests/x86.rs`,
built once with the pinned tools (MSVC 14.44.35207, Windows SDK
10.0.19041.0, `bin/Hostx64/x86`); the bytes are kept, not rebuilt by the
workspace build.

```
cl /nologo /c /TC /O1 /GS- /Zl /W4 /WX /X /Brepro <SDK and MSVC include dirs> x86dll.c
link /NOLOGO /NODEFAULTLIB /DLL /ENTRY:DllMain@12 /MACHINE:X86 /DYNAMICBASE /NXCOMPAT
     /SAFESEH /Brepro /DEBUG:NONE /INCREMENTAL:NO /MANIFEST:NO /OPT:REF /BASE:0x10000000
     /DEF:x86dll.def /OUT:x86dll.dll /IMPLIB:x86dll.lib x86dll.obj
ml /nologo /c /safeseh handler.asm
cl /nologo /c /TC /O1 /GS- /Zl /W4 /WX /X /Brepro <include dirs> x86app.c
link /NOLOGO /NODEFAULTLIB /ENTRY:entry /SUBSYSTEM:CONSOLE,6.01 /MACHINE:X86
     /DYNAMICBASE /NXCOMPAT /SAFESEH /Brepro /DEBUG:NONE /INCREMENTAL:NO /MANIFEST:NO
     /OPT:REF /BASE:0x400000 /OUT:x86app.exe x86app.obj handler.obj x86dll.lib
```

What `dumpbin` shows: x86app.exe imports `DoorAdd` by name and ordinal 2
from x86dll.dll, has 17 HIGHLOW relocations (x86dll.dll 3), a TLS directory
(template 0x22AC..0x22B0, index 0x3008, one callback), and a load
configuration with its security cookie at 0x3000 holding the x86 default
0xBB40E64E and a SafeSEH table of one handler at 0x2138.
