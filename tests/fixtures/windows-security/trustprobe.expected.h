/* Generated from Windows 11 26H2's output of `trustprobe.exe raw` on the owner's PC
 * (one string per compared step). */
    /* files */
    "signed-good.exe none=0x800b010a chain-cached=0x800b010a\r\n"
    "signed-revoked.exe none=0x800b010a chain-cached=0x800b010a\r\n"
    "hello.exe none=0x800b0100 chain-cached=0x800b0100\r\n"
    "readme.txt none=0x800b0003 chain-cached=0x800b0003\r\n"
    "missing.exe none=0x80092003 chain-cached=0x80092003\r\n"
    "bad-choice=0x00000057 unknown-action=0x800b0001\r\n"
,
    /* hashes */
    "acquire ok ok sha384 error=0x80090008\r\n"
    "hello.exe sha1=b9f5acc0abc955361bb383561feec21dfc8131d2 sha256=03879f52657095d5666dfe22a8eba63fcb8efa3ab2102875fc73d7c0f0c69137\r\n"
    "signed-good.exe sha1=b9f5acc0abc955361bb383561feec21dfc8131d2 sha256=03879f52657095d5666dfe22a8eba63fcb8efa3ab2102875fc73d7c0f0c69137\r\n"
    "readme.txt sha1=a4e574683d76883c90b2718c4c3ce58bcd93f71c sha256=61e09723c8f7da5d36c020622cb0b87b659324b1bdb350e542fda2ad8fcf993e\r\n"
    "short ok=0 error=0x0000007a size=32 query ok=1 error=0x00000000 size=20\r\n"
,
    /* catalog */
    "mode=0 hello=0x800b010a readme=0x800b010a sha1-tag=0x80096010 signed-copy=0x800b010a\r\n"
    "wrong-file=0x80096010 unknown-tag=0x800b0100 not-a-catalog=0x80092003 missing-catalog=0x80092003\r\n"
    "mode=1 hello=0x800b010a readme=0x800b010a sha1-tag=0x80096010 signed-copy=0x800b010a\r\n"
    "wrong-file=0x80096010 unknown-tag=0x800b0100 not-a-catalog=0x80092003 missing-catalog=0x80092003\r\n"
    "mode=2 hello=0x800b0100 readme=0x800b0100 sha1-tag=0x800b0100 signed-copy=0x800b0100\r\n"
    "wrong-file=0x800b0100 unknown-tag=0x800b0100 not-a-catalog=0x80092003 missing-catalog=0x80092003\r\n"
    "mode=3 hello=0x800b0100 readme=0x800b0100 sha1-tag=0x800b0100 signed-copy=0x800b0100\r\n"
    "wrong-file=0x800b0100 unknown-tag=0x800b0100 not-a-catalog=0x80092003 missing-catalog=0x80092003\r\n"
    "mode=4 hello=0x800b010a readme=0x800b010a sha1-tag=0x800b010a signed-copy=0x800b010a\r\n"
    "wrong-file=0x800b010a unknown-tag=0x800b010a not-a-catalog=0x80092003 missing-catalog=0x80092003\r\n"
    "mode=5 hello=0x800b010a readme=0x800b010a sha1-tag=0x800b010a signed-copy=0x800b010a\r\n"
    "wrong-file=0x800b010a unknown-tag=0x800b010a not-a-catalog=0x80092003 missing-catalog=0x80092003\r\n"
