; Inno Setup script: out\Tandem-PDF-<version>-windows-x64-setup.exe from the packaged build (run `npm run package` first).
;   iscc /DVersion=0.1.0 scripts\installer.iss
; A per-user install with no admin prompt, to the same place and with the same Start menu shortcut and "Open with"
; entries as scripts\install-windows.ps1 (keep the two in step). The default PDF app is left as it is.
#define Name "Tandem PDF"
#define ProgId "TandemPDF.Document"

[Setup]
AppId={{6B0B6E0E-6C4B-4F0A-9A53-2D0C5B7C7F41}
AppName={#Name}
AppVersion={#Version}
AppPublisher=Shiro-Legacy
AppPublisherURL=https://github.com/Shiro-Legacy/TPDF
DefaultDirName={localappdata}\Programs\{#Name}
PrivilegesRequired=lowest
DisableDirPage=yes
DisableProgramGroupPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
SourceDir=..
OutputDir=out
OutputBaseFilename=Tandem-PDF-{#Version}-windows-x64-setup
SetupIconFile=assets\icon.ico
UninstallDisplayIcon={app}\{#Name}.exe
Compression=lzma2
SolidCompression=yes
WizardStyle=modern

[Files]
Source: "out\{#Name}-win32-x64\*"; DestDir: "{app}"; Flags: recursesubdirs ignoreversion

[Icons]
Name: "{userprograms}\{#Name}"; Filename: "{app}\{#Name}.exe"

[Registry]
Root: HKCU; Subkey: "Software\Classes\{#ProgId}"; ValueType: string; ValueData: "PDF document"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\{#ProgId}\DefaultIcon"; ValueType: string; ValueData: """{app}\{#Name}.exe"",0"
Root: HKCU; Subkey: "Software\Classes\{#ProgId}\shell\open\command"; ValueType: string; ValueData: """{app}\{#Name}.exe"" ""%1"""
Root: HKCU; Subkey: "Software\Classes\Applications\{#Name}.exe"; ValueType: string; ValueName: "FriendlyAppName"; ValueData: "{#Name}"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\Applications\{#Name}.exe\shell\open\command"; ValueType: string; ValueData: """{app}\{#Name}.exe"" ""%1"""
Root: HKCU; Subkey: "Software\Classes\Applications\{#Name}.exe\SupportedTypes"; ValueType: string; ValueName: ".pdf"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\.pdf\OpenWithProgids"; ValueType: none; ValueName: "{#ProgId}"; Flags: uninsdeletevalue

[Run]
Filename: "{app}\{#Name}.exe"; Description: "Open {#Name}"; Flags: nowait postinstall skipifsilent
