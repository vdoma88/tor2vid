; Установщик Vibewall (Inno Setup 6). Ставит программу без прав
; администратора в %LOCALAPPDATA%\Programs\Vibewall. Версию передаёт сборка:
;   ISCC /DAppVersion=0.2.0 packaging\vibewall.iss
; Файлы берутся из dist\vibewall (корень репозитория), установщик кладётся в dist.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
AppId={{7E3C1A52-6B1D-4F0E-9B8A-5D2C9E4F1A37}
AppName=Vibewall
AppVersion={#AppVersion}
AppVerName=Vibewall {#AppVersion}
AppPublisher=Vibewall
AppPublisherURL=https://github.com/vdoma88/tor2vid
AppSupportURL=https://github.com/vdoma88/tor2vid/issues
AppUpdatesURL=https://github.com/vdoma88/tor2vid/releases
DefaultDirName={localappdata}\Programs\Vibewall
DefaultGroupName=Vibewall
DisableProgramGroupPage=yes
DisableDirPage=auto
PrivilegesRequired=lowest
OutputDir=..\..\dist
OutputBaseFilename=VibewallSetup-{#AppVersion}
SetupIconFile=..\assets\icon.ico
UninstallDisplayIcon={app}\vibewall.exe
UninstallDisplayName=Vibewall
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
CloseApplications=force
RestartApplications=no
MinVersion=10.0

[Languages]
Name: "ru"; MessagesFile: "compiler:Languages\Russian.isl"
Name: "en"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
Source: "..\..\dist\vibewall\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs

[Icons]
Name: "{autoprograms}\Vibewall"; Filename: "{app}\vibewall.exe"
Name: "{autodesktop}\Vibewall"; Filename: "{app}\vibewall.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\vibewall.exe"; Description: "{cm:LaunchProgram,Vibewall}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
; Программа живёт в трее — перед удалением её надо закрыть.
Filename: "{sys}\taskkill.exe"; Parameters: "/F /IM vibewall.exe"; Flags: runhidden; RunOnceId: "StopVibewall"

[Registry]
; Автозапуск программа включает сама; при удалении запись убираем.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueName: "Vibewall"; Flags: uninsdeletevalue dontcreatekey
