//! Documented Win32 constant values (WinUser.h / WinGDI.h / CommCtrl.h /
//! CommDlg.h). Grouped by prefix so callers write `wm::PAINT`, `ws::CHILD`.
#![allow(missing_docs)]

pub mod wm {
    pub const NULL: u32 = 0x0000;
    pub const CREATE: u32 = 0x0001;
    pub const DESTROY: u32 = 0x0002;
    pub const MOVE: u32 = 0x0003;
    pub const SIZE: u32 = 0x0005;
    pub const ACTIVATE: u32 = 0x0006;
    pub const SETFOCUS: u32 = 0x0007;
    pub const KILLFOCUS: u32 = 0x0008;
    pub const ENABLE: u32 = 0x000a;
    pub const SETREDRAW: u32 = 0x000b;
    pub const SETTEXT: u32 = 0x000c;
    pub const GETTEXT: u32 = 0x000d;
    pub const GETTEXTLENGTH: u32 = 0x000e;
    pub const PAINT: u32 = 0x000f;
    pub const CLOSE: u32 = 0x0010;
    pub const QUERYENDSESSION: u32 = 0x0011;
    pub const QUIT: u32 = 0x0012;
    pub const ERASEBKGND: u32 = 0x0014;
    pub const SYSCOLORCHANGE: u32 = 0x0015;
    pub const ENDSESSION: u32 = 0x0016;
    pub const SHOWWINDOW: u32 = 0x0018;
    pub const SETTINGCHANGE: u32 = 0x001a;
    pub const ACTIVATEAPP: u32 = 0x001c;
    pub const CANCELMODE: u32 = 0x001f;
    pub const SETCURSOR: u32 = 0x0020;
    pub const MOUSEACTIVATE: u32 = 0x0021;
    pub const CHILDACTIVATE: u32 = 0x0022;
    pub const GETMINMAXINFO: u32 = 0x0024;
    pub const NEXTDLGCTL: u32 = 0x0028;
    pub const DRAWITEM: u32 = 0x002b;
    pub const MEASUREITEM: u32 = 0x002c;
    pub const DELETEITEM: u32 = 0x002d;
    pub const VKEYTOITEM: u32 = 0x002e;
    pub const CHARTOITEM: u32 = 0x002f;
    pub const SETFONT: u32 = 0x0030;
    pub const GETFONT: u32 = 0x0031;
    pub const COMPAREITEM: u32 = 0x0039;
    pub const WINDOWPOSCHANGING: u32 = 0x0046;
    pub const WINDOWPOSCHANGED: u32 = 0x0047;
    pub const NOTIFY: u32 = 0x004e;
    pub const INPUTLANGCHANGE: u32 = 0x0051;
    pub const HELP: u32 = 0x0053;
    pub const NOTIFYFORMAT: u32 = 0x0055;
    pub const CONTEXTMENU: u32 = 0x007b;
    pub const STYLECHANGING: u32 = 0x007c;
    pub const STYLECHANGED: u32 = 0x007d;
    pub const DISPLAYCHANGE: u32 = 0x007e;
    pub const GETICON: u32 = 0x007f;
    pub const SETICON: u32 = 0x0080;
    pub const NCCREATE: u32 = 0x0081;
    pub const NCDESTROY: u32 = 0x0082;
    pub const NCCALCSIZE: u32 = 0x0083;
    pub const NCHITTEST: u32 = 0x0084;
    pub const NCPAINT: u32 = 0x0085;
    pub const NCACTIVATE: u32 = 0x0086;
    pub const GETDLGCODE: u32 = 0x0087;
    pub const NCMOUSEMOVE: u32 = 0x00a0;
    pub const NCLBUTTONDOWN: u32 = 0x00a1;
    pub const NCLBUTTONUP: u32 = 0x00a2;
    pub const NCLBUTTONDBLCLK: u32 = 0x00a3;
    pub const NCRBUTTONDOWN: u32 = 0x00a4;
    pub const NCRBUTTONUP: u32 = 0x00a5;
    pub const NCMBUTTONDOWN: u32 = 0x00a7;
    pub const NCMBUTTONUP: u32 = 0x00a8;
    pub const KEYFIRST: u32 = 0x0100;
    pub const KEYDOWN: u32 = 0x0100;
    pub const KEYUP: u32 = 0x0101;
    pub const CHAR: u32 = 0x0102;
    pub const DEADCHAR: u32 = 0x0103;
    pub const SYSKEYDOWN: u32 = 0x0104;
    pub const SYSKEYUP: u32 = 0x0105;
    pub const SYSCHAR: u32 = 0x0106;
    pub const SYSDEADCHAR: u32 = 0x0107;
    pub const UNICHAR: u32 = 0x0109;
    pub const KEYLAST: u32 = 0x0109;
    pub const IME_STARTCOMPOSITION: u32 = 0x010d;
    pub const IME_ENDCOMPOSITION: u32 = 0x010e;
    pub const IME_COMPOSITION: u32 = 0x010f;
    pub const INITDIALOG: u32 = 0x0110;
    pub const COMMAND: u32 = 0x0111;
    pub const SYSCOMMAND: u32 = 0x0112;
    pub const TIMER: u32 = 0x0113;
    pub const HSCROLL: u32 = 0x0114;
    pub const VSCROLL: u32 = 0x0115;
    pub const INITMENU: u32 = 0x0116;
    pub const INITMENUPOPUP: u32 = 0x0117;
    pub const MENUSELECT: u32 = 0x011f;
    pub const MENUCHAR: u32 = 0x0120;
    pub const ENTERIDLE: u32 = 0x0121;
    pub const UNINITMENUPOPUP: u32 = 0x0125;
    pub const CHANGEUISTATE: u32 = 0x0127;
    pub const UPDATEUISTATE: u32 = 0x0128;
    pub const QUERYUISTATE: u32 = 0x0129;
    pub const CTLCOLORMSGBOX: u32 = 0x0132;
    pub const CTLCOLOREDIT: u32 = 0x0133;
    pub const CTLCOLORLISTBOX: u32 = 0x0134;
    pub const CTLCOLORBTN: u32 = 0x0135;
    pub const CTLCOLORDLG: u32 = 0x0136;
    pub const CTLCOLORSCROLLBAR: u32 = 0x0137;
    pub const CTLCOLORSTATIC: u32 = 0x0138;
    pub const MOUSEFIRST: u32 = 0x0200;
    pub const MOUSEMOVE: u32 = 0x0200;
    pub const LBUTTONDOWN: u32 = 0x0201;
    pub const LBUTTONUP: u32 = 0x0202;
    pub const LBUTTONDBLCLK: u32 = 0x0203;
    pub const RBUTTONDOWN: u32 = 0x0204;
    pub const RBUTTONUP: u32 = 0x0205;
    pub const RBUTTONDBLCLK: u32 = 0x0206;
    pub const MBUTTONDOWN: u32 = 0x0207;
    pub const MBUTTONUP: u32 = 0x0208;
    pub const MBUTTONDBLCLK: u32 = 0x0209;
    pub const MOUSEWHEEL: u32 = 0x020a;
    pub const MOUSEHWHEEL: u32 = 0x020e;
    pub const MOUSELAST: u32 = 0x020e;
    pub const PARENTNOTIFY: u32 = 0x0210;
    pub const ENTERMENULOOP: u32 = 0x0211;
    pub const EXITMENULOOP: u32 = 0x0212;
    pub const SIZING: u32 = 0x0214;
    pub const CAPTURECHANGED: u32 = 0x0215;
    pub const MOVING: u32 = 0x0216;
    pub const ENTERSIZEMOVE: u32 = 0x0231;
    pub const EXITSIZEMOVE: u32 = 0x0232;
    pub const DROPFILES: u32 = 0x0233;
    pub const MOUSEHOVER: u32 = 0x02a1;
    pub const MOUSELEAVE: u32 = 0x02a3;
    pub const DPICHANGED: u32 = 0x02e0;
    pub const CUT: u32 = 0x0300;
    pub const COPY: u32 = 0x0301;
    pub const PASTE: u32 = 0x0302;
    pub const CLEAR: u32 = 0x0303;
    pub const UNDO: u32 = 0x0304;
    pub const PRINTCLIENT: u32 = 0x0318;
    pub const THEMECHANGED: u32 = 0x031a;
    pub const USER: u32 = 0x0400;
    pub const APP: u32 = 0x8000;
}

pub mod ws {
    pub const OVERLAPPED: u32 = 0x0000_0000;
    pub const TABSTOP: u32 = 0x0001_0000;
    pub const MAXIMIZEBOX: u32 = 0x0001_0000;
    pub const GROUP: u32 = 0x0002_0000;
    pub const MINIMIZEBOX: u32 = 0x0002_0000;
    pub const THICKFRAME: u32 = 0x0004_0000;
    pub const SYSMENU: u32 = 0x0008_0000;
    pub const HSCROLL: u32 = 0x0010_0000;
    pub const VSCROLL: u32 = 0x0020_0000;
    pub const DLGFRAME: u32 = 0x0040_0000;
    pub const BORDER: u32 = 0x0080_0000;
    pub const CAPTION: u32 = 0x00c0_0000;
    pub const MAXIMIZE: u32 = 0x0100_0000;
    pub const CLIPCHILDREN: u32 = 0x0200_0000;
    pub const CLIPSIBLINGS: u32 = 0x0400_0000;
    pub const DISABLED: u32 = 0x0800_0000;
    pub const VISIBLE: u32 = 0x1000_0000;
    pub const MINIMIZE: u32 = 0x2000_0000;
    pub const CHILD: u32 = 0x4000_0000;
    pub const POPUP: u32 = 0x8000_0000;
    pub const OVERLAPPEDWINDOW: u32 =
        OVERLAPPED | CAPTION | SYSMENU | THICKFRAME | MINIMIZEBOX | MAXIMIZEBOX;
    pub const POPUPWINDOW: u32 = POPUP | BORDER | SYSMENU;
}

pub mod ws_ex {
    pub const DLGMODALFRAME: u32 = 0x0000_0001;
    pub const NOPARENTNOTIFY: u32 = 0x0000_0004;
    pub const TOPMOST: u32 = 0x0000_0008;
    pub const ACCEPTFILES: u32 = 0x0000_0010;
    pub const TRANSPARENT: u32 = 0x0000_0020;
    pub const MDICHILD: u32 = 0x0000_0040;
    pub const TOOLWINDOW: u32 = 0x0000_0080;
    pub const WINDOWEDGE: u32 = 0x0000_0100;
    pub const CLIENTEDGE: u32 = 0x0000_0200;
    pub const CONTEXTHELP: u32 = 0x0000_0400;
    pub const RIGHT: u32 = 0x0000_1000;
    pub const RTLREADING: u32 = 0x0000_2000;
    pub const LEFTSCROLLBAR: u32 = 0x0000_4000;
    pub const CONTROLPARENT: u32 = 0x0001_0000;
    pub const STATICEDGE: u32 = 0x0002_0000;
    pub const APPWINDOW: u32 = 0x0004_0000;
    pub const LAYERED: u32 = 0x0008_0000;
    pub const NOINHERITLAYOUT: u32 = 0x0010_0000;
    pub const LAYOUTRTL: u32 = 0x0040_0000;
    pub const COMPOSITED: u32 = 0x0200_0000;
    pub const NOACTIVATE: u32 = 0x0800_0000;
    pub const OVERLAPPEDWINDOW: u32 = WINDOWEDGE | CLIENTEDGE;
}

pub mod cs {
    pub const VREDRAW: u32 = 0x0001;
    pub const HREDRAW: u32 = 0x0002;
    pub const DBLCLKS: u32 = 0x0008;
    pub const OWNDC: u32 = 0x0020;
    pub const CLASSDC: u32 = 0x0040;
    pub const PARENTDC: u32 = 0x0080;
    pub const NOCLOSE: u32 = 0x0200;
    pub const SAVEBITS: u32 = 0x0800;
    pub const BYTEALIGNCLIENT: u32 = 0x1000;
    pub const BYTEALIGNWINDOW: u32 = 0x2000;
    pub const GLOBALCLASS: u32 = 0x4000;
    pub const DROPSHADOW: u32 = 0x0002_0000;
}

pub mod sw {
    pub const HIDE: i32 = 0;
    pub const SHOWNORMAL: i32 = 1;
    pub const SHOWMINIMIZED: i32 = 2;
    pub const SHOWMAXIMIZED: i32 = 3;
    pub const SHOWNOACTIVATE: i32 = 4;
    pub const SHOW: i32 = 5;
    pub const MINIMIZE: i32 = 6;
    pub const SHOWMINNOACTIVE: i32 = 7;
    pub const SHOWNA: i32 = 8;
    pub const RESTORE: i32 = 9;
    pub const SHOWDEFAULT: i32 = 10;
    pub const FORCEMINIMIZE: i32 = 11;
}

pub mod swp {
    pub const NOSIZE: u32 = 0x0001;
    pub const NOMOVE: u32 = 0x0002;
    pub const NOZORDER: u32 = 0x0004;
    pub const NOREDRAW: u32 = 0x0008;
    pub const NOACTIVATE: u32 = 0x0010;
    pub const FRAMECHANGED: u32 = 0x0020;
    pub const SHOWWINDOW: u32 = 0x0040;
    pub const HIDEWINDOW: u32 = 0x0080;
    pub const NOCOPYBITS: u32 = 0x0100;
    pub const NOOWNERZORDER: u32 = 0x0200;
    pub const NOSENDCHANGING: u32 = 0x0400;
    pub const DEFERERASE: u32 = 0x2000;
    pub const ASYNCWINDOWPOS: u32 = 0x4000;
    pub const STATECHANGED: u32 = 0x8000;
    /// Every documented flag; anything else is rejected.
    pub const VALID: u32 = 0xe7ff;
    /// HWND_* insertion pseudo-handles.
    pub const HWND_TOP: isize = 0;
    pub const HWND_BOTTOM: isize = 1;
    pub const HWND_TOPMOST: isize = -1;
    pub const HWND_NOTOPMOST: isize = -2;
    pub const HWND_MESSAGE: isize = -3;
    pub const HWND_BROADCAST: isize = 0xffff;
}

pub mod ht {
    pub const ERROR: i32 = -2;
    pub const TRANSPARENT: i32 = -1;
    pub const NOWHERE: i32 = 0;
    pub const CLIENT: i32 = 1;
    pub const CAPTION: i32 = 2;
    pub const SYSMENU: i32 = 3;
    pub const GROWBOX: i32 = 4;
    pub const MENU: i32 = 5;
    pub const HSCROLL: i32 = 6;
    pub const VSCROLL: i32 = 7;
    pub const MINBUTTON: i32 = 8;
    pub const MAXBUTTON: i32 = 9;
    pub const LEFT: i32 = 10;
    pub const RIGHT: i32 = 11;
    pub const TOP: i32 = 12;
    pub const TOPLEFT: i32 = 13;
    pub const TOPRIGHT: i32 = 14;
    pub const BOTTOM: i32 = 15;
    pub const BOTTOMLEFT: i32 = 16;
    pub const BOTTOMRIGHT: i32 = 17;
    pub const BORDER: i32 = 18;
    pub const CLOSE: i32 = 20;
    pub const HELP: i32 = 21;
}

pub mod sc {
    pub const SIZE: usize = 0xf000;
    pub const MOVE: usize = 0xf010;
    pub const MINIMIZE: usize = 0xf020;
    pub const MAXIMIZE: usize = 0xf030;
    pub const NEXTWINDOW: usize = 0xf040;
    pub const PREVWINDOW: usize = 0xf050;
    pub const CLOSE: usize = 0xf060;
    pub const VSCROLL: usize = 0xf070;
    pub const HSCROLL: usize = 0xf080;
    pub const MOUSEMENU: usize = 0xf090;
    pub const KEYMENU: usize = 0xf100;
    pub const RESTORE: usize = 0xf120;
    pub const TASKLIST: usize = 0xf130;
    pub const CONTEXTHELP: usize = 0xf180;
}

pub mod mk {
    pub const LBUTTON: usize = 0x0001;
    pub const RBUTTON: usize = 0x0002;
    pub const SHIFT: usize = 0x0004;
    pub const CONTROL: usize = 0x0008;
    pub const MBUTTON: usize = 0x0010;
}

/// Button styles, notifications, messages and states.
pub mod bs {
    pub const PUSHBUTTON: u32 = 0x0;
    pub const DEFPUSHBUTTON: u32 = 0x1;
    pub const CHECKBOX: u32 = 0x2;
    pub const AUTOCHECKBOX: u32 = 0x3;
    pub const RADIOBUTTON: u32 = 0x4;
    pub const STATE3: u32 = 0x5;
    pub const AUTO3STATE: u32 = 0x6;
    pub const GROUPBOX: u32 = 0x7;
    pub const USERBUTTON: u32 = 0x8;
    pub const AUTORADIOBUTTON: u32 = 0x9;
    pub const PUSHBOX: u32 = 0xa;
    pub const OWNERDRAW: u32 = 0xb;
    pub const TYPEMASK: u32 = 0xf;
    pub const LEFTTEXT: u32 = 0x20;
    pub const TEXT: u32 = 0x0;
    pub const ICON: u32 = 0x40;
    pub const BITMAP: u32 = 0x80;
    pub const LEFT: u32 = 0x100;
    pub const RIGHT: u32 = 0x200;
    pub const CENTER: u32 = 0x300;
    pub const TOP: u32 = 0x400;
    pub const BOTTOM: u32 = 0x800;
    pub const VCENTER: u32 = 0xc00;
    pub const PUSHLIKE: u32 = 0x1000;
    pub const MULTILINE: u32 = 0x2000;
    pub const NOTIFY: u32 = 0x4000;
    pub const FLAT: u32 = 0x8000;
}
pub mod bn {
    pub const CLICKED: u32 = 0;
    pub const PAINT: u32 = 1;
    pub const PUSHED: u32 = 2;
    pub const UNPUSHED: u32 = 3;
    pub const DISABLE: u32 = 4;
    pub const DBLCLK: u32 = 5;
    pub const SETFOCUS: u32 = 6;
    pub const KILLFOCUS: u32 = 7;
}
pub mod bm {
    pub const GETCHECK: u32 = 0x00f0;
    pub const SETCHECK: u32 = 0x00f1;
    pub const GETSTATE: u32 = 0x00f2;
    pub const SETSTATE: u32 = 0x00f3;
    pub const SETSTYLE: u32 = 0x00f4;
    pub const CLICK: u32 = 0x00f5;
    pub const GETIMAGE: u32 = 0x00f6;
    pub const SETIMAGE: u32 = 0x00f7;
    pub const SETDONTCLICK: u32 = 0x00f8;
}
pub mod bst {
    pub const UNCHECKED: usize = 0x0;
    pub const CHECKED: usize = 0x1;
    pub const INDETERMINATE: usize = 0x2;
    pub const PUSHED: usize = 0x4;
    pub const FOCUS: usize = 0x8;
    pub const HOT: usize = 0x200;
}

pub mod es {
    pub const LEFT: u32 = 0x0000;
    pub const CENTER: u32 = 0x0001;
    pub const RIGHT: u32 = 0x0002;
    pub const MULTILINE: u32 = 0x0004;
    pub const UPPERCASE: u32 = 0x0008;
    pub const LOWERCASE: u32 = 0x0010;
    pub const PASSWORD: u32 = 0x0020;
    pub const AUTOVSCROLL: u32 = 0x0040;
    pub const AUTOHSCROLL: u32 = 0x0080;
    pub const NOHIDESEL: u32 = 0x0100;
    pub const OEMCONVERT: u32 = 0x0400;
    pub const READONLY: u32 = 0x0800;
    pub const WANTRETURN: u32 = 0x1000;
    pub const NUMBER: u32 = 0x2000;
}
pub mod en {
    pub const SETFOCUS: u32 = 0x0100;
    pub const KILLFOCUS: u32 = 0x0200;
    pub const CHANGE: u32 = 0x0300;
    pub const UPDATE: u32 = 0x0400;
    pub const ERRSPACE: u32 = 0x0500;
    pub const MAXTEXT: u32 = 0x0501;
    pub const HSCROLL: u32 = 0x0601;
    pub const VSCROLL: u32 = 0x0602;
}
pub mod em {
    pub const GETSEL: u32 = 0x00b0;
    pub const SETSEL: u32 = 0x00b1;
    pub const GETRECT: u32 = 0x00b2;
    pub const SETRECT: u32 = 0x00b3;
    pub const SETRECTNP: u32 = 0x00b4;
    pub const SCROLL: u32 = 0x00b5;
    pub const LINESCROLL: u32 = 0x00b6;
    pub const SCROLLCARET: u32 = 0x00b7;
    pub const GETMODIFY: u32 = 0x00b8;
    pub const SETMODIFY: u32 = 0x00b9;
    pub const GETLINECOUNT: u32 = 0x00ba;
    pub const LINEINDEX: u32 = 0x00bb;
    pub const SETHANDLE: u32 = 0x00bc;
    pub const GETHANDLE: u32 = 0x00bd;
    pub const GETTHUMB: u32 = 0x00be;
    pub const LINELENGTH: u32 = 0x00c1;
    pub const REPLACESEL: u32 = 0x00c2;
    pub const GETLINE: u32 = 0x00c4;
    pub const LIMITTEXT: u32 = 0x00c5;
    pub const SETLIMITTEXT: u32 = 0x00c5;
    pub const CANUNDO: u32 = 0x00c6;
    pub const UNDO: u32 = 0x00c7;
    pub const FMTLINES: u32 = 0x00c8;
    pub const LINEFROMCHAR: u32 = 0x00c9;
    pub const SETTABSTOPS: u32 = 0x00cb;
    pub const SETPASSWORDCHAR: u32 = 0x00cc;
    pub const EMPTYUNDOBUFFER: u32 = 0x00cd;
    pub const GETFIRSTVISIBLELINE: u32 = 0x00ce;
    pub const SETREADONLY: u32 = 0x00cf;
    pub const SETWORDBREAKPROC: u32 = 0x00d0;
    pub const GETWORDBREAKPROC: u32 = 0x00d1;
    pub const GETPASSWORDCHAR: u32 = 0x00d2;
    pub const SETMARGINS: u32 = 0x00d3;
    pub const GETMARGINS: u32 = 0x00d4;
    pub const GETLIMITTEXT: u32 = 0x00d5;
    pub const POSFROMCHAR: u32 = 0x00d6;
    pub const CHARFROMPOS: u32 = 0x00d7;
    pub const EC_LEFTMARGIN: usize = 0x0001;
    pub const EC_RIGHTMARGIN: usize = 0x0002;
    pub const EC_USEFONTINFO: usize = 0xffff;
}

pub mod ss {
    pub const LEFT: u32 = 0x00;
    pub const CENTER: u32 = 0x01;
    pub const RIGHT: u32 = 0x02;
    pub const ICON: u32 = 0x03;
    pub const BLACKRECT: u32 = 0x04;
    pub const GRAYRECT: u32 = 0x05;
    pub const WHITERECT: u32 = 0x06;
    pub const BLACKFRAME: u32 = 0x07;
    pub const GRAYFRAME: u32 = 0x08;
    pub const WHITEFRAME: u32 = 0x09;
    pub const USERITEM: u32 = 0x0a;
    pub const SIMPLE: u32 = 0x0b;
    pub const LEFTNOWORDWRAP: u32 = 0x0c;
    pub const OWNERDRAW: u32 = 0x0d;
    pub const BITMAP: u32 = 0x0e;
    pub const ENHMETAFILE: u32 = 0x0f;
    pub const ETCHEDHORZ: u32 = 0x10;
    pub const ETCHEDVERT: u32 = 0x11;
    pub const ETCHEDFRAME: u32 = 0x12;
    pub const TYPEMASK: u32 = 0x1f;
    pub const NOPREFIX: u32 = 0x80;
    pub const NOTIFY: u32 = 0x100;
    pub const CENTERIMAGE: u32 = 0x200;
    pub const RIGHTJUST: u32 = 0x400;
    pub const SUNKEN: u32 = 0x1000;
    pub const EDITCONTROL: u32 = 0x2000;
    pub const ENDELLIPSIS: u32 = 0x4000;
    pub const PATHELLIPSIS: u32 = 0x8000;
    pub const WORDELLIPSIS: u32 = 0xc000;
}
pub mod stn {
    pub const CLICKED: u32 = 0;
    pub const DBLCLK: u32 = 1;
    pub const ENABLE: u32 = 2;
    pub const DISABLE: u32 = 3;
}
pub mod stm {
    pub const SETICON: u32 = 0x0170;
    pub const GETICON: u32 = 0x0171;
    pub const SETIMAGE: u32 = 0x0172;
    pub const GETIMAGE: u32 = 0x0173;
}

pub mod lbs {
    pub const NOTIFY: u32 = 0x0001;
    pub const SORT: u32 = 0x0002;
    pub const NOREDRAW: u32 = 0x0004;
    pub const MULTIPLESEL: u32 = 0x0008;
    pub const OWNERDRAWFIXED: u32 = 0x0010;
    pub const OWNERDRAWVARIABLE: u32 = 0x0020;
    pub const HASSTRINGS: u32 = 0x0040;
    pub const USETABSTOPS: u32 = 0x0080;
    pub const NOINTEGRALHEIGHT: u32 = 0x0100;
    pub const MULTICOLUMN: u32 = 0x0200;
    pub const WANTKEYBOARDINPUT: u32 = 0x0400;
    pub const EXTENDEDSEL: u32 = 0x0800;
    pub const DISABLENOSCROLL: u32 = 0x1000;
    pub const NODATA: u32 = 0x2000;
    pub const NOSEL: u32 = 0x4000;
    pub const COMBOBOX: u32 = 0x8000;
    pub const STANDARD: u32 = NOTIFY | SORT | 0x0020_0000 | 0x0080_0000;
}
pub mod lbn {
    pub const ERRSPACE: i32 = -2;
    pub const SELCHANGE: u32 = 1;
    pub const DBLCLK: u32 = 2;
    pub const SELCANCEL: u32 = 3;
    pub const SETFOCUS: u32 = 4;
    pub const KILLFOCUS: u32 = 5;
}
pub mod lb {
    pub const OKAY: isize = 0;
    pub const ERR: isize = -1;
    pub const ERRSPACE: isize = -2;
    pub const ADDSTRING: u32 = 0x0180;
    pub const INSERTSTRING: u32 = 0x0181;
    pub const DELETESTRING: u32 = 0x0182;
    pub const SELITEMRANGEEX: u32 = 0x0183;
    pub const RESETCONTENT: u32 = 0x0184;
    pub const SETSEL: u32 = 0x0185;
    pub const SETCURSEL: u32 = 0x0186;
    pub const GETSEL: u32 = 0x0187;
    pub const GETCURSEL: u32 = 0x0188;
    pub const GETTEXT: u32 = 0x0189;
    pub const GETTEXTLEN: u32 = 0x018a;
    pub const GETCOUNT: u32 = 0x018b;
    pub const SELECTSTRING: u32 = 0x018c;
    pub const DIR: u32 = 0x018d;
    pub const GETTOPINDEX: u32 = 0x018e;
    pub const FINDSTRING: u32 = 0x018f;
    pub const GETSELCOUNT: u32 = 0x0190;
    pub const GETSELITEMS: u32 = 0x0191;
    pub const SETTABSTOPS: u32 = 0x0192;
    pub const GETHORIZONTALEXTENT: u32 = 0x0193;
    pub const SETHORIZONTALEXTENT: u32 = 0x0194;
    pub const SETCOLUMNWIDTH: u32 = 0x0195;
    pub const ADDFILE: u32 = 0x0196;
    pub const SETTOPINDEX: u32 = 0x0197;
    pub const GETITEMRECT: u32 = 0x0198;
    pub const GETITEMDATA: u32 = 0x0199;
    pub const SETITEMDATA: u32 = 0x019a;
    pub const SELITEMRANGE: u32 = 0x019b;
    pub const SETANCHORINDEX: u32 = 0x019c;
    pub const GETANCHORINDEX: u32 = 0x019d;
    pub const SETCARETINDEX: u32 = 0x019e;
    pub const GETCARETINDEX: u32 = 0x019f;
    pub const SETITEMHEIGHT: u32 = 0x01a0;
    pub const GETITEMHEIGHT: u32 = 0x01a1;
    pub const FINDSTRINGEXACT: u32 = 0x01a2;
    pub const SETLOCALE: u32 = 0x01a5;
    pub const GETLOCALE: u32 = 0x01a6;
    pub const SETCOUNT: u32 = 0x01a7;
    pub const INITSTORAGE: u32 = 0x01a8;
    pub const ITEMFROMPOINT: u32 = 0x01a9;
}

pub mod cbs {
    pub const SIMPLE: u32 = 0x0001;
    pub const DROPDOWN: u32 = 0x0002;
    pub const DROPDOWNLIST: u32 = 0x0003;
    pub const OWNERDRAWFIXED: u32 = 0x0010;
    pub const OWNERDRAWVARIABLE: u32 = 0x0020;
    pub const AUTOHSCROLL: u32 = 0x0040;
    pub const OEMCONVERT: u32 = 0x0080;
    pub const SORT: u32 = 0x0100;
    pub const HASSTRINGS: u32 = 0x0200;
    pub const NOINTEGRALHEIGHT: u32 = 0x0400;
    pub const DISABLENOSCROLL: u32 = 0x0800;
    pub const UPPERCASE: u32 = 0x2000;
    pub const LOWERCASE: u32 = 0x4000;
}
pub mod cbn {
    pub const ERRSPACE: i32 = -1;
    pub const SELCHANGE: u32 = 1;
    pub const DBLCLK: u32 = 2;
    pub const SETFOCUS: u32 = 3;
    pub const KILLFOCUS: u32 = 4;
    pub const EDITCHANGE: u32 = 5;
    pub const EDITUPDATE: u32 = 6;
    pub const DROPDOWN: u32 = 7;
    pub const CLOSEUP: u32 = 8;
    pub const SELENDOK: u32 = 9;
    pub const SELENDCANCEL: u32 = 10;
}
pub mod cb {
    pub const OKAY: isize = 0;
    pub const ERR: isize = -1;
    pub const ERRSPACE: isize = -2;
    pub const GETEDITSEL: u32 = 0x0140;
    pub const LIMITTEXT: u32 = 0x0141;
    pub const SETEDITSEL: u32 = 0x0142;
    pub const ADDSTRING: u32 = 0x0143;
    pub const DELETESTRING: u32 = 0x0144;
    pub const DIR: u32 = 0x0145;
    pub const GETCOUNT: u32 = 0x0146;
    pub const GETCURSEL: u32 = 0x0147;
    pub const GETLBTEXT: u32 = 0x0148;
    pub const GETLBTEXTLEN: u32 = 0x0149;
    pub const INSERTSTRING: u32 = 0x014a;
    pub const RESETCONTENT: u32 = 0x014b;
    pub const FINDSTRING: u32 = 0x014c;
    pub const SELECTSTRING: u32 = 0x014d;
    pub const SETCURSEL: u32 = 0x014e;
    pub const SHOWDROPDOWN: u32 = 0x014f;
    pub const GETITEMDATA: u32 = 0x0150;
    pub const SETITEMDATA: u32 = 0x0151;
    pub const GETDROPPEDCONTROLRECT: u32 = 0x0152;
    pub const SETITEMHEIGHT: u32 = 0x0153;
    pub const GETITEMHEIGHT: u32 = 0x0154;
    pub const SETEXTENDEDUI: u32 = 0x0155;
    pub const GETEXTENDEDUI: u32 = 0x0156;
    pub const GETDROPPEDSTATE: u32 = 0x0157;
    pub const FINDSTRINGEXACT: u32 = 0x0158;
    pub const SETLOCALE: u32 = 0x0159;
    pub const GETLOCALE: u32 = 0x015a;
    pub const GETTOPINDEX: u32 = 0x015b;
    pub const SETTOPINDEX: u32 = 0x015c;
    pub const GETHORIZONTALEXTENT: u32 = 0x015d;
    pub const SETHORIZONTALEXTENT: u32 = 0x015e;
    pub const GETDROPPEDWIDTH: u32 = 0x015f;
    pub const SETDROPPEDWIDTH: u32 = 0x0160;
    pub const INITSTORAGE: u32 = 0x0161;
    pub const GETCOMBOBOXINFO: u32 = 0x0164;
}

pub mod sbs {
    pub const HORZ: u32 = 0x0000;
    pub const VERT: u32 = 0x0001;
    pub const TOPALIGN: u32 = 0x0002;
    pub const LEFTALIGN: u32 = 0x0002;
    pub const BOTTOMALIGN: u32 = 0x0004;
    pub const RIGHTALIGN: u32 = 0x0004;
    pub const SIZEBOXTOPLEFTALIGN: u32 = 0x0002;
    pub const SIZEBOXBOTTOMRIGHTALIGN: u32 = 0x0004;
    pub const SIZEBOX: u32 = 0x0008;
    pub const SIZEGRIP: u32 = 0x0010;
}
/// Scroll bar identifiers, codes and `SCROLLINFO` masks.
pub mod sb {
    pub const HORZ: i32 = 0;
    pub const VERT: i32 = 1;
    pub const CTL: i32 = 2;
    pub const BOTH: i32 = 3;
    pub const LINEUP: u32 = 0;
    pub const LINELEFT: u32 = 0;
    pub const LINEDOWN: u32 = 1;
    pub const LINERIGHT: u32 = 1;
    pub const PAGEUP: u32 = 2;
    pub const PAGELEFT: u32 = 2;
    pub const PAGEDOWN: u32 = 3;
    pub const PAGERIGHT: u32 = 3;
    pub const THUMBPOSITION: u32 = 4;
    pub const THUMBTRACK: u32 = 5;
    pub const TOP: u32 = 6;
    pub const LEFT: u32 = 6;
    pub const BOTTOM: u32 = 7;
    pub const RIGHT: u32 = 7;
    pub const ENDSCROLL: u32 = 8;
    pub const SIF_RANGE: u32 = 0x0001;
    pub const SIF_PAGE: u32 = 0x0002;
    pub const SIF_POS: u32 = 0x0004;
    pub const SIF_DISABLENOSCROLL: u32 = 0x0008;
    pub const SIF_TRACKPOS: u32 = 0x0010;
    pub const SIF_ALL: u32 = SIF_RANGE | SIF_PAGE | SIF_POS | SIF_TRACKPOS;
    pub const ESB_ENABLE_BOTH: u32 = 0;
    pub const ESB_DISABLE_LTUP: u32 = 1;
    pub const ESB_DISABLE_RTDN: u32 = 2;
    pub const ESB_DISABLE_BOTH: u32 = 3;
}
pub mod sbm {
    pub const SETPOS: u32 = 0x00e0;
    pub const GETPOS: u32 = 0x00e1;
    pub const SETRANGE: u32 = 0x00e2;
    pub const SETRANGEREDRAW: u32 = 0x00e6;
    pub const GETRANGE: u32 = 0x00e3;
    pub const ENABLE_ARROWS: u32 = 0x00e4;
    pub const SETSCROLLINFO: u32 = 0x00e9;
    pub const GETSCROLLINFO: u32 = 0x00ea;
    pub const GETSCROLLBARINFO: u32 = 0x00eb;
}

pub mod ds {
    pub const ABSALIGN: u32 = 0x01;
    pub const SYSMODAL: u32 = 0x02;
    pub const LOCALEDIT: u32 = 0x20;
    pub const SETFONT: u32 = 0x40;
    pub const MODALFRAME: u32 = 0x80;
    pub const NOIDLEMSG: u32 = 0x100;
    pub const SETFOREGROUND: u32 = 0x200;
    pub const THREE_D_LOOK: u32 = 0x0004;
    pub const FIXEDSYS: u32 = 0x0008;
    pub const NOFAILCREATE: u32 = 0x0010;
    pub const CONTROL: u32 = 0x0400;
    pub const CENTER: u32 = 0x0800;
    pub const CENTERMOUSE: u32 = 0x1000;
    pub const CONTEXTHELP: u32 = 0x2000;
    pub const SHELLFONT: u32 = SETFONT | FIXEDSYS;
}
pub mod dm {
    pub const GETDEFID: u32 = 0x0400;
    pub const SETDEFID: u32 = 0x0401;
    pub const REPOSITION: u32 = 0x0402;
    pub const DC_HASDEFID: usize = 0x534b;
}
pub mod dlgc {
    pub const WANTARROWS: u32 = 0x0001;
    pub const WANTTAB: u32 = 0x0002;
    pub const WANTALLKEYS: u32 = 0x0004;
    pub const WANTMESSAGE: u32 = 0x0004;
    pub const HASSETSEL: u32 = 0x0008;
    pub const DEFPUSHBUTTON: u32 = 0x0010;
    pub const UNDEFPUSHBUTTON: u32 = 0x0020;
    pub const RADIOBUTTON: u32 = 0x0040;
    pub const WANTCHARS: u32 = 0x0080;
    pub const STATIC: u32 = 0x0100;
    pub const BUTTON: u32 = 0x2000;
}
/// The dialog window-extra slots (x64 offsets; user32 places them a
/// pointer apart in each build, P17).
pub mod dwlp {
    pub const MSGRESULT: i32 = 0;
    pub const DLGPROC: i32 = 8;
    pub const USER: i32 = 16;
}

pub mod mf {
    pub const INSERT: u32 = 0x0000;
    pub const CHANGE: u32 = 0x0080;
    pub const APPEND: u32 = 0x0100;
    pub const DELETE: u32 = 0x0200;
    pub const REMOVE: u32 = 0x1000;
    pub const BYCOMMAND: u32 = 0x0000;
    pub const BYPOSITION: u32 = 0x0400;
    pub const SEPARATOR: u32 = 0x0800;
    pub const ENABLED: u32 = 0x0000;
    pub const GRAYED: u32 = 0x0001;
    pub const DISABLED: u32 = 0x0002;
    pub const UNCHECKED: u32 = 0x0000;
    pub const CHECKED: u32 = 0x0008;
    pub const USECHECKBITMAPS: u32 = 0x0200;
    pub const STRING: u32 = 0x0000;
    pub const BITMAP: u32 = 0x0004;
    pub const OWNERDRAW: u32 = 0x0100;
    pub const POPUP: u32 = 0x0010;
    pub const MENUBARBREAK: u32 = 0x0020;
    pub const MENUBREAK: u32 = 0x0040;
    pub const UNHILITE: u32 = 0x0000;
    pub const HILITE: u32 = 0x0080;
    pub const DEFAULT: u32 = 0x1000;
    pub const SYSMENU: u32 = 0x2000;
    pub const HELP: u32 = 0x4000;
    pub const RIGHTJUSTIFY: u32 = 0x4000;
    pub const MOUSESELECT: u32 = 0x8000;
    pub const END: u32 = 0x0080;
}
pub mod mft {
    pub const STRING: u32 = 0x0000;
    pub const BITMAP: u32 = 0x0004;
    pub const MENUBARBREAK: u32 = 0x0020;
    pub const MENUBREAK: u32 = 0x0040;
    pub const OWNERDRAW: u32 = 0x0100;
    pub const RADIOCHECK: u32 = 0x0200;
    pub const SEPARATOR: u32 = 0x0800;
    pub const RIGHTORDER: u32 = 0x2000;
    pub const RIGHTJUSTIFY: u32 = 0x4000;
}
pub mod mfs {
    pub const GRAYED: u32 = 0x0003;
    pub const DISABLED: u32 = 0x0003;
    pub const CHECKED: u32 = 0x0008;
    pub const HILITE: u32 = 0x0080;
    pub const ENABLED: u32 = 0x0000;
    pub const UNCHECKED: u32 = 0x0000;
    pub const UNHILITE: u32 = 0x0000;
    pub const DEFAULT: u32 = 0x1000;
}
pub mod miim {
    pub const STATE: u32 = 0x0001;
    pub const ID: u32 = 0x0002;
    pub const SUBMENU: u32 = 0x0004;
    pub const CHECKMARKS: u32 = 0x0008;
    pub const TYPE: u32 = 0x0010;
    pub const DATA: u32 = 0x0020;
    pub const STRING: u32 = 0x0040;
    pub const BITMAP: u32 = 0x0080;
    pub const FTYPE: u32 = 0x0100;
}
pub mod tpm {
    pub const LEFTBUTTON: u32 = 0x0000;
    pub const RIGHTBUTTON: u32 = 0x0002;
    pub const LEFTALIGN: u32 = 0x0000;
    pub const CENTERALIGN: u32 = 0x0004;
    pub const RIGHTALIGN: u32 = 0x0008;
    pub const TOPALIGN: u32 = 0x0000;
    pub const VCENTERALIGN: u32 = 0x0010;
    pub const BOTTOMALIGN: u32 = 0x0020;
    pub const HORIZONTAL: u32 = 0x0000;
    pub const VERTICAL: u32 = 0x0040;
    pub const NONOTIFY: u32 = 0x0080;
    pub const RETURNCMD: u32 = 0x0100;
    pub const RECURSE: u32 = 0x0001;
    pub const HORPOSANIMATION: u32 = 0x0400;
    pub const HORNEGANIMATION: u32 = 0x0800;
    pub const VERPOSANIMATION: u32 = 0x1000;
    pub const VERNEGANIMATION: u32 = 0x2000;
    pub const NOANIMATION: u32 = 0x4000;
    pub const LAYOUTRTL: u32 = 0x8000;
    pub const WORKAREA: u32 = 0x10000;
}

/// Dialog command identifiers.
pub mod id {
    pub const OK: u32 = 1;
    pub const CANCEL: u32 = 2;
    pub const ABORT: u32 = 3;
    pub const RETRY: u32 = 4;
    pub const IGNORE: u32 = 5;
    pub const YES: u32 = 6;
    pub const NO: u32 = 7;
    pub const CLOSE: u32 = 8;
    pub const HELP: u32 = 9;
    pub const TRYAGAIN: u32 = 10;
    pub const CONTINUE: u32 = 11;
    pub const TIMEOUT: u32 = 32000;
}
pub mod mb {
    pub const OK: u32 = 0x0;
    pub const OKCANCEL: u32 = 0x1;
    pub const ABORTRETRYIGNORE: u32 = 0x2;
    pub const YESNOCANCEL: u32 = 0x3;
    pub const YESNO: u32 = 0x4;
    pub const RETRYCANCEL: u32 = 0x5;
    pub const CANCELTRYCONTINUE: u32 = 0x6;
    pub const TYPEMASK: u32 = 0xf;
    pub const ICONHAND: u32 = 0x10;
    pub const ICONQUESTION: u32 = 0x20;
    pub const ICONEXCLAMATION: u32 = 0x30;
    pub const ICONASTERISK: u32 = 0x40;
    pub const USERICON: u32 = 0x80;
    pub const ICONMASK: u32 = 0xf0;
    pub const DEFBUTTON1: u32 = 0x000;
    pub const DEFBUTTON2: u32 = 0x100;
    pub const DEFBUTTON3: u32 = 0x200;
    pub const DEFBUTTON4: u32 = 0x300;
    pub const DEFMASK: u32 = 0xf00;
    pub const APPLMODAL: u32 = 0x0000;
    pub const SYSTEMMODAL: u32 = 0x1000;
    pub const TASKMODAL: u32 = 0x2000;
    pub const MODEMASK: u32 = 0x3000;
    pub const HELP: u32 = 0x4000;
    pub const NOFOCUS: u32 = 0x8000;
    pub const SETFOREGROUND: u32 = 0x10000;
    pub const DEFAULT_DESKTOP_ONLY: u32 = 0x20000;
    pub const TOPMOST: u32 = 0x40000;
    pub const RIGHT: u32 = 0x80000;
    pub const RTLREADING: u32 = 0x100000;
    pub const SERVICE_NOTIFICATION: u32 = 0x200000;
}

/// `GetWindowLongPtr` / `GetClassLongPtr` indices.
pub mod gwl {
    pub const WNDPROC: i32 = -4;
    pub const HINSTANCE: i32 = -6;
    pub const HWNDPARENT: i32 = -8;
    pub const STYLE: i32 = -16;
    pub const EXSTYLE: i32 = -20;
    pub const USERDATA: i32 = -21;
    pub const ID: i32 = -12;
}
pub mod gcl {
    pub const MENUNAME: i32 = -8;
    pub const HBRBACKGROUND: i32 = -10;
    pub const HCURSOR: i32 = -12;
    pub const HICON: i32 = -14;
    pub const HMODULE: i32 = -16;
    pub const CBWNDEXTRA: i32 = -18;
    pub const CBCLSEXTRA: i32 = -20;
    pub const WNDPROC: i32 = -24;
    pub const STYLE: i32 = -26;
    pub const ATOM: i32 = -32;
    pub const HICONSM: i32 = -34;
}
pub mod gw {
    pub const HWNDFIRST: u32 = 0;
    pub const HWNDLAST: u32 = 1;
    pub const HWNDNEXT: u32 = 2;
    pub const HWNDPREV: u32 = 3;
    pub const OWNER: u32 = 4;
    pub const CHILD: u32 = 5;
    pub const ENABLEDPOPUP: u32 = 6;
    pub const GA_PARENT: u32 = 1;
    pub const GA_ROOT: u32 = 2;
    pub const GA_ROOTOWNER: u32 = 3;
}

/// PeekMessage/GetQueueStatus flags.
pub mod pm {
    pub const NOREMOVE: u32 = 0x0000;
    pub const REMOVE: u32 = 0x0001;
    pub const NOYIELD: u32 = 0x0002;
    pub const QS_INPUT_FILTER: u32 = 0x0407 << 16;
    pub const QS_POSTMESSAGE_FILTER: u32 = (0x0008 | 0x0080 | 0x0010) << 16;
    pub const QS_PAINT_FILTER: u32 = 0x0020 << 16;
    pub const QS_SENDMESSAGE_FILTER: u32 = 0x0040 << 16;
}
pub mod qs {
    pub const KEY: u32 = 0x0001;
    pub const MOUSEMOVE: u32 = 0x0002;
    pub const MOUSEBUTTON: u32 = 0x0004;
    pub const POSTMESSAGE: u32 = 0x0008;
    pub const TIMER: u32 = 0x0010;
    pub const PAINT: u32 = 0x0020;
    pub const SENDMESSAGE: u32 = 0x0040;
    pub const HOTKEY: u32 = 0x0080;
    pub const ALLPOSTMESSAGE: u32 = 0x0100;
    pub const RAWINPUT: u32 = 0x0400;
    pub const MOUSE: u32 = MOUSEMOVE | MOUSEBUTTON;
    pub const INPUT: u32 = MOUSE | KEY | RAWINPUT;
    pub const ALLEVENTS: u32 = INPUT | POSTMESSAGE | TIMER | PAINT | HOTKEY;
    pub const ALLINPUT: u32 = ALLEVENTS | SENDMESSAGE;
}
pub mod wa {
    pub const INACTIVE: usize = 0;
    pub const ACTIVE: usize = 1;
    pub const CLICKACTIVE: usize = 2;
}
pub mod size {
    pub const RESTORED: usize = 0;
    pub const MINIMIZED: usize = 1;
    pub const MAXIMIZED: usize = 2;
    pub const MAXSHOW: usize = 3;
    pub const MAXHIDE: usize = 4;
}
pub mod ma {
    pub const ACTIVATE: isize = 1;
    pub const ACTIVATEANDEAT: isize = 2;
    pub const NOACTIVATE: isize = 3;
    pub const NOACTIVATEANDEAT: isize = 4;
}
/// `DrawText` / `DrawTextEx` formats.
pub mod dt {
    pub const TOP: u32 = 0x0000;
    pub const LEFT: u32 = 0x0000;
    pub const CENTER: u32 = 0x0001;
    pub const RIGHT: u32 = 0x0002;
    pub const VCENTER: u32 = 0x0004;
    pub const BOTTOM: u32 = 0x0008;
    pub const WORDBREAK: u32 = 0x0010;
    pub const SINGLELINE: u32 = 0x0020;
    pub const EXPANDTABS: u32 = 0x0040;
    pub const TABSTOP: u32 = 0x0080;
    pub const NOCLIP: u32 = 0x0100;
    pub const EXTERNALLEADING: u32 = 0x0200;
    pub const CALCRECT: u32 = 0x0400;
    pub const NOPREFIX: u32 = 0x0800;
    pub const INTERNAL: u32 = 0x1000;
    pub const EDITCONTROL: u32 = 0x2000;
    pub const PATH_ELLIPSIS: u32 = 0x4000;
    pub const END_ELLIPSIS: u32 = 0x8000;
    pub const MODIFYSTRING: u32 = 0x10000;
    pub const RTLREADING: u32 = 0x20000;
    pub const WORD_ELLIPSIS: u32 = 0x40000;
    pub const NOFULLWIDTHCHARBREAK: u32 = 0x80000;
    pub const HIDEPREFIX: u32 = 0x100000;
    pub const PREFIXONLY: u32 = 0x200000;
}
/// `DrawEdge` edge and border flags.
pub mod edge {
    pub const BDR_RAISEDOUTER: u32 = 0x0001;
    pub const BDR_SUNKENOUTER: u32 = 0x0002;
    pub const BDR_RAISEDINNER: u32 = 0x0004;
    pub const BDR_SUNKENINNER: u32 = 0x0008;
    pub const RAISED: u32 = BDR_RAISEDOUTER | BDR_RAISEDINNER;
    pub const SUNKEN: u32 = BDR_SUNKENOUTER | BDR_SUNKENINNER;
    pub const ETCHED: u32 = BDR_SUNKENOUTER | BDR_RAISEDINNER;
    pub const BUMP: u32 = BDR_RAISEDOUTER | BDR_SUNKENINNER;
    pub const BF_LEFT: u32 = 0x0001;
    pub const BF_TOP: u32 = 0x0002;
    pub const BF_RIGHT: u32 = 0x0004;
    pub const BF_BOTTOM: u32 = 0x0008;
    pub const BF_RECT: u32 = 0x000f;
    pub const BF_DIAGONAL: u32 = 0x0010;
    pub const BF_MIDDLE: u32 = 0x0800;
    pub const BF_SOFT: u32 = 0x1000;
    pub const BF_ADJUST: u32 = 0x2000;
    pub const BF_FLAT: u32 = 0x4000;
    pub const BF_MONO: u32 = 0x8000;
}
/// `DrawFrameControl` types and states.
pub mod dfc {
    pub const CAPTION: u32 = 1;
    pub const MENU: u32 = 2;
    pub const SCROLL: u32 = 3;
    pub const BUTTON: u32 = 4;
    pub const DFCS_BUTTONCHECK: u32 = 0x0000;
    pub const DFCS_BUTTONRADIO: u32 = 0x0004;
    pub const DFCS_BUTTON3STATE: u32 = 0x0008;
    pub const DFCS_BUTTONPUSH: u32 = 0x0010;
    pub const DFCS_SCROLLUP: u32 = 0x0000;
    pub const DFCS_SCROLLDOWN: u32 = 0x0001;
    pub const DFCS_SCROLLLEFT: u32 = 0x0002;
    pub const DFCS_SCROLLRIGHT: u32 = 0x0003;
    pub const DFCS_SCROLLCOMBOBOX: u32 = 0x0005;
    pub const DFCS_MENUARROW: u32 = 0x0000;
    pub const DFCS_MENUCHECK: u32 = 0x0001;
    pub const DFCS_MENUBULLET: u32 = 0x0002;
    pub const DFCS_INACTIVE: u32 = 0x0100;
    pub const DFCS_PUSHED: u32 = 0x0200;
    pub const DFCS_CHECKED: u32 = 0x0400;
    pub const DFCS_FLAT: u32 = 0x4000;
    pub const DFCS_MONO: u32 = 0x8000;
}
/// `GetSysColor` indices.
pub mod color {
    pub const SCROLLBAR: i32 = 0;
    pub const BACKGROUND: i32 = 1;
    pub const ACTIVECAPTION: i32 = 2;
    pub const INACTIVECAPTION: i32 = 3;
    pub const MENU: i32 = 4;
    pub const WINDOW: i32 = 5;
    pub const WINDOWFRAME: i32 = 6;
    pub const MENUTEXT: i32 = 7;
    pub const WINDOWTEXT: i32 = 8;
    pub const CAPTIONTEXT: i32 = 9;
    pub const ACTIVEBORDER: i32 = 10;
    pub const INACTIVEBORDER: i32 = 11;
    pub const APPWORKSPACE: i32 = 12;
    pub const HIGHLIGHT: i32 = 13;
    pub const HIGHLIGHTTEXT: i32 = 14;
    pub const BTNFACE: i32 = 15;
    pub const BTNSHADOW: i32 = 16;
    pub const GRAYTEXT: i32 = 17;
    pub const BTNTEXT: i32 = 18;
    pub const INACTIVECAPTIONTEXT: i32 = 19;
    pub const BTNHIGHLIGHT: i32 = 20;
    pub const DKSHADOW_3D: i32 = 21;
    pub const LIGHT_3D: i32 = 22;
    pub const INFOTEXT: i32 = 23;
    pub const INFOBK: i32 = 24;
    pub const HOTLIGHT: i32 = 26;
    pub const GRADIENTACTIVECAPTION: i32 = 27;
    pub const GRADIENTINACTIVECAPTION: i32 = 28;
    pub const MENUHILIGHT: i32 = 29;
    pub const MENUBAR: i32 = 30;
    pub const COUNT: usize = 31;
}
/// Predefined cursors and icons (`MAKEINTRESOURCE` ordinals).
pub mod idc {
    pub const ARROW: u16 = 32512;
    pub const IBEAM: u16 = 32513;
    pub const WAIT: u16 = 32514;
    pub const CROSS: u16 = 32515;
    pub const UPARROW: u16 = 32516;
    pub const SIZE: u16 = 32640;
    pub const ICON: u16 = 32641;
    pub const SIZENWSE: u16 = 32642;
    pub const SIZENESW: u16 = 32643;
    pub const SIZEWE: u16 = 32644;
    pub const SIZENS: u16 = 32645;
    pub const SIZEALL: u16 = 32646;
    pub const NO: u16 = 32648;
    pub const HAND: u16 = 32649;
    pub const APPSTARTING: u16 = 32650;
    pub const HELP: u16 = 32651;
    pub const PIN: u16 = 32671;
    pub const PERSON: u16 = 32672;
}
pub mod idi {
    pub const APPLICATION: u16 = 32512;
    pub const HAND: u16 = 32513;
    pub const ERROR: u16 = 32513;
    pub const QUESTION: u16 = 32514;
    pub const EXCLAMATION: u16 = 32515;
    pub const WARNING: u16 = 32515;
    pub const ASTERISK: u16 = 32516;
    pub const INFORMATION: u16 = 32516;
    pub const WINLOGO: u16 = 32517;
    pub const SHIELD: u16 = 32518;
}
/// Resource types.
pub mod rt {
    pub const CURSOR: u16 = 1;
    pub const BITMAP: u16 = 2;
    pub const ICON: u16 = 3;
    pub const MENU: u16 = 4;
    pub const DIALOG: u16 = 5;
    pub const STRING: u16 = 6;
    pub const ACCELERATOR: u16 = 9;
    pub const GROUP_CURSOR: u16 = 12;
    pub const GROUP_ICON: u16 = 14;
    pub const MANIFEST: u16 = 24;
}
/// Accelerator flags (`ACCEL.fVirt`).
pub mod fvirt {
    pub const VIRTKEY: u8 = 0x01;
    pub const NOINVERT: u8 = 0x02;
    pub const SHIFT: u8 = 0x04;
    pub const CONTROL: u8 = 0x08;
    pub const ALT: u8 = 0x10;
    /// Resource-only end-of-table marker.
    pub const END: u8 = 0x80;
}
/// UI state (`WM_CHANGEUISTATE`).
pub mod uis {
    pub const SET: u16 = 1;
    pub const CLEAR: u16 = 2;
    pub const INITIALIZE: u16 = 3;
    pub const UISF_HIDEFOCUS: u16 = 0x1;
    pub const UISF_HIDEACCEL: u16 = 0x2;
}

// --------------------------------------------------------------------------
// GDI
// --------------------------------------------------------------------------

/// Binary raster operations (`SetROP2`), 1..=16.
pub mod r2 {
    pub const BLACK: u32 = 1;
    pub const NOTMERGEPEN: u32 = 2;
    pub const MASKNOTPEN: u32 = 3;
    pub const NOTCOPYPEN: u32 = 4;
    pub const MASKPENNOT: u32 = 5;
    pub const NOT: u32 = 6;
    pub const XORPEN: u32 = 7;
    pub const NOTMASKPEN: u32 = 8;
    pub const MASKPEN: u32 = 9;
    pub const NOTXORPEN: u32 = 10;
    pub const NOP: u32 = 11;
    pub const MERGENOTPEN: u32 = 12;
    pub const COPYPEN: u32 = 13;
    pub const MERGEPENNOT: u32 = 14;
    pub const MERGEPEN: u32 = 15;
    pub const WHITE: u32 = 16;
}
/// Ternary raster operations (high byte of the 3rd word is the truth table).
pub mod rop {
    pub const SRCCOPY: u32 = 0x00cc_0020;
    pub const SRCPAINT: u32 = 0x00ee_0086;
    pub const SRCAND: u32 = 0x0088_00c6;
    pub const SRCINVERT: u32 = 0x0066_0046;
    pub const SRCERASE: u32 = 0x0044_0328;
    pub const NOTSRCCOPY: u32 = 0x0033_0008;
    pub const NOTSRCERASE: u32 = 0x0011_00a6;
    pub const MERGECOPY: u32 = 0x00c0_00ca;
    pub const MERGEPAINT: u32 = 0x00bb_0226;
    pub const PATCOPY: u32 = 0x00f0_0021;
    pub const PATPAINT: u32 = 0x00fb_0a09;
    pub const PATINVERT: u32 = 0x005a_0049;
    pub const DSTINVERT: u32 = 0x0055_0009;
    pub const BLACKNESS: u32 = 0x0000_0042;
    pub const WHITENESS: u32 = 0x00ff_0062;
    pub const NOMIRRORBITMAP: u32 = 0x8000_0000;
    pub const CAPTUREBLT: u32 = 0x4000_0000;
}
pub mod stock {
    pub const WHITE_BRUSH: i32 = 0;
    pub const LTGRAY_BRUSH: i32 = 1;
    pub const GRAY_BRUSH: i32 = 2;
    pub const DKGRAY_BRUSH: i32 = 3;
    pub const BLACK_BRUSH: i32 = 4;
    pub const NULL_BRUSH: i32 = 5;
    pub const WHITE_PEN: i32 = 6;
    pub const BLACK_PEN: i32 = 7;
    pub const NULL_PEN: i32 = 8;
    pub const OEM_FIXED_FONT: i32 = 10;
    pub const ANSI_FIXED_FONT: i32 = 11;
    pub const ANSI_VAR_FONT: i32 = 12;
    pub const SYSTEM_FONT: i32 = 13;
    pub const DEVICE_DEFAULT_FONT: i32 = 14;
    pub const DEFAULT_PALETTE: i32 = 15;
    pub const SYSTEM_FIXED_FONT: i32 = 16;
    pub const DEFAULT_GUI_FONT: i32 = 17;
    pub const DC_BRUSH: i32 = 18;
    pub const DC_PEN: i32 = 19;
}
/// Pen styles.
pub mod ps {
    pub const SOLID: u32 = 0;
    pub const DASH: u32 = 1;
    pub const DOT: u32 = 2;
    pub const DASHDOT: u32 = 3;
    pub const DASHDOTDOT: u32 = 4;
    pub const NULL: u32 = 5;
    pub const INSIDEFRAME: u32 = 6;
    pub const USERSTYLE: u32 = 7;
    pub const ALTERNATE: u32 = 8;
    pub const STYLE_MASK: u32 = 0xf;
    pub const ENDCAP_ROUND: u32 = 0x000;
    pub const ENDCAP_SQUARE: u32 = 0x100;
    pub const ENDCAP_FLAT: u32 = 0x200;
    pub const ENDCAP_MASK: u32 = 0xf00;
    pub const JOIN_ROUND: u32 = 0x0000;
    pub const JOIN_BEVEL: u32 = 0x1000;
    pub const JOIN_MITER: u32 = 0x2000;
    pub const JOIN_MASK: u32 = 0xf000;
    pub const COSMETIC: u32 = 0x0000_0000;
    pub const GEOMETRIC: u32 = 0x0001_0000;
    pub const TYPE_MASK: u32 = 0x000f_0000;
}
/// Brush styles (`LOGBRUSH.lbStyle`).
pub mod brush {
    pub const SOLID: u32 = 0;
    pub const NULL: u32 = 1;
    pub const HOLLOW: u32 = 1;
    pub const HATCHED: u32 = 2;
    pub const PATTERN: u32 = 3;
    pub const INDEXED: u32 = 4;
    pub const DIBPATTERN: u32 = 5;
    pub const DIBPATTERNPT: u32 = 6;
    pub const PATTERN8X8: u32 = 7;
    pub const DIBPATTERN8X8: u32 = 8;
}
pub mod hs {
    pub const HORIZONTAL: u32 = 0;
    pub const VERTICAL: u32 = 1;
    pub const FDIAGONAL: u32 = 2;
    pub const BDIAGONAL: u32 = 3;
    pub const CROSS: u32 = 4;
    pub const DIAGCROSS: u32 = 5;
}
pub mod ta {
    pub const NOUPDATECP: u32 = 0;
    pub const UPDATECP: u32 = 1;
    pub const LEFT: u32 = 0;
    pub const RIGHT: u32 = 2;
    pub const CENTER: u32 = 6;
    pub const TOP: u32 = 0;
    pub const BOTTOM: u32 = 8;
    pub const BASELINE: u32 = 24;
    pub const RTLREADING: u32 = 256;
    pub const MASK: u32 = BASELINE | CENTER | UPDATECP | RTLREADING;
}
pub mod eto {
    pub const OPAQUE: u32 = 0x0002;
    pub const CLIPPED: u32 = 0x0004;
    pub const GLYPH_INDEX: u32 = 0x0010;
    pub const RTLREADING: u32 = 0x0080;
    pub const NUMERICSLOCAL: u32 = 0x0400;
    pub const NUMERICSLATIN: u32 = 0x0800;
    pub const IGNORELANGUAGE: u32 = 0x1000;
    pub const PDY: u32 = 0x2000;
}
pub mod bk {
    pub const TRANSPARENT: i32 = 1;
    pub const OPAQUE: i32 = 2;
}
pub mod mm {
    pub const TEXT: i32 = 1;
    pub const LOMETRIC: i32 = 2;
    pub const HIMETRIC: i32 = 3;
    pub const LOENGLISH: i32 = 4;
    pub const HIENGLISH: i32 = 5;
    pub const TWIPS: i32 = 6;
    pub const ISOTROPIC: i32 = 7;
    pub const ANISOTROPIC: i32 = 8;
}
pub mod rgn {
    pub const ERROR: i32 = 0;
    pub const NULLREGION: i32 = 1;
    pub const SIMPLEREGION: i32 = 2;
    pub const COMPLEXREGION: i32 = 3;
    pub const AND: i32 = 1;
    pub const OR: i32 = 2;
    pub const XOR: i32 = 3;
    pub const DIFF: i32 = 4;
    pub const COPY: i32 = 5;
}
pub mod obj {
    pub const PEN: u32 = 1;
    pub const BRUSH: u32 = 2;
    pub const DC: u32 = 3;
    pub const METADC: u32 = 4;
    pub const PAL: u32 = 5;
    pub const FONT: u32 = 6;
    pub const BITMAP: u32 = 7;
    pub const REGION: u32 = 8;
    pub const METAFILE: u32 = 9;
    pub const MEMDC: u32 = 10;
    pub const EXTPEN: u32 = 11;
    pub const ENHMETADC: u32 = 12;
    pub const ENHMETAFILE: u32 = 13;
    pub const COLORSPACE: u32 = 14;
}
pub mod bi {
    pub const RGB: u32 = 0;
    pub const RLE8: u32 = 1;
    pub const RLE4: u32 = 2;
    pub const BITFIELDS: u32 = 3;
    pub const JPEG: u32 = 4;
    pub const PNG: u32 = 5;
    pub const DIB_RGB_COLORS: u32 = 0;
    pub const DIB_PAL_COLORS: u32 = 1;
    pub const CBM_INIT: u32 = 4;
}
/// `GetDeviceCaps` indices.
pub mod caps {
    pub const DRIVERVERSION: i32 = 0;
    pub const TECHNOLOGY: i32 = 2;
    pub const HORZSIZE: i32 = 4;
    pub const VERTSIZE: i32 = 6;
    pub const HORZRES: i32 = 8;
    pub const VERTRES: i32 = 10;
    pub const BITSPIXEL: i32 = 12;
    pub const PLANES: i32 = 14;
    pub const NUMBRUSHES: i32 = 16;
    pub const NUMPENS: i32 = 18;
    pub const NUMMARKERS: i32 = 20;
    pub const NUMFONTS: i32 = 22;
    pub const NUMCOLORS: i32 = 24;
    pub const PDEVICESIZE: i32 = 26;
    pub const CURVECAPS: i32 = 28;
    pub const LINECAPS: i32 = 30;
    pub const POLYGONALCAPS: i32 = 32;
    pub const TEXTCAPS: i32 = 34;
    pub const CLIPCAPS: i32 = 36;
    pub const RASTERCAPS: i32 = 38;
    pub const ASPECTX: i32 = 40;
    pub const ASPECTY: i32 = 42;
    pub const ASPECTXY: i32 = 44;
    pub const LOGPIXELSX: i32 = 88;
    pub const LOGPIXELSY: i32 = 90;
    pub const SIZEPALETTE: i32 = 104;
    pub const NUMRESERVED: i32 = 106;
    pub const COLORRES: i32 = 108;
    pub const PHYSICALWIDTH: i32 = 110;
    pub const PHYSICALHEIGHT: i32 = 111;
    pub const PHYSICALOFFSETX: i32 = 112;
    pub const PHYSICALOFFSETY: i32 = 113;
    pub const VREFRESH: i32 = 116;
    pub const DESKTOPVERTRES: i32 = 117;
    pub const DESKTOPHORZRES: i32 = 118;
    pub const BLTALIGNMENT: i32 = 119;
    pub const SHADEBLENDCAPS: i32 = 120;
    pub const COLORMGMTCAPS: i32 = 121;
    pub const DT_RASDISPLAY: i32 = 1;
    pub const RC_BITBLT: i32 = 1;
    pub const RC_BITMAP64: i32 = 8;
    pub const RC_DI_BITMAP: i32 = 0x80;
    pub const RC_DIBTODEV: i32 = 0x200;
    pub const RC_STRETCHBLT: i32 = 0x800;
    pub const RC_STRETCHDIB: i32 = 0x2000;
    pub const CC_CIRCLES: i32 = 1;
    pub const CC_ELLIPSES: i32 = 8;
    pub const CC_INTERIORS: i32 = 128;
    pub const LC_POLYLINE: i32 = 2;
    pub const LC_WIDE: i32 = 16;
    pub const LC_STYLED: i32 = 32;
    pub const LC_INTERIORS: i32 = 128;
    pub const PC_POLYGON: i32 = 1;
    pub const PC_RECTANGLE: i32 = 2;
    pub const PC_WINDPOLYGON: i32 = 4;
    pub const PC_INTERIORS: i32 = 128;
    pub const CP_RECTANGLE: i32 = 1;
    pub const CP_REGION: i32 = 2;
    pub const TC_CR_ANY: i32 = 0x0010;
    pub const TC_RA_ABLE: i32 = 0x2000;
}
pub mod fw {
    pub const DONTCARE: i32 = 0;
    pub const THIN: i32 = 100;
    pub const LIGHT: i32 = 300;
    pub const NORMAL: i32 = 400;
    pub const MEDIUM: i32 = 500;
    pub const SEMIBOLD: i32 = 600;
    pub const BOLD: i32 = 700;
    pub const HEAVY: i32 = 900;
}
pub mod stretch {
    pub const BLACKONWHITE: i32 = 1;
    pub const WHITEONBLACK: i32 = 2;
    pub const COLORONCOLOR: i32 = 3;
    pub const HALFTONE: i32 = 4;
}
pub mod fill {
    pub const ALTERNATE: i32 = 1;
    pub const WINDING: i32 = 2;
}
