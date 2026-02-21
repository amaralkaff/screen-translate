#define MyAppName "Screen Translate"
#define MyAppExeName "screen-translate.exe"
#define MyAppPublisher "amaralkaff"
#define MyAppURL "https://github.com/amaralkaff/screen-translate"

; Version injected from CI: ISCC /DMyAppVersion=x.y.z installer.iss
#ifndef MyAppVersion
  #define MyAppVersion "0.0.0"
#endif

[Setup]
AppId={{E3A7F1B2-9C4D-4E5F-8A6B-1D2E3F4A5B6C}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}/issues
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
PrivilegesRequired=lowest
OutputDir=installer-output
OutputBaseFilename=ScreenTranslate-{#MyAppVersion}-setup
SetupIconFile=assets\icon.ico
UninstallDisplayIcon={app}\{#MyAppExeName}
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; GroupDescription: "Additional shortcuts:"

[Files]
Source: "target\release\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{group}\Uninstall {#MyAppName}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "{#MyAppName}"; ValueData: """{app}\{#MyAppExeName}"""; Flags: uninsdeletevalue

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "Launch {#MyAppName}"; Flags: nowait postinstall skipifsilent
Filename: "{app}\{#MyAppExeName}"; Flags: nowait skipifnotsilent

[UninstallDelete]
Type: filesandordirs; Name: "{app}\libretranslate"

[UninstallRun]
Filename: "taskkill"; Parameters: "/F /IM {#MyAppExeName}"; Flags: runhidden; RunOnceId: "KillApp"

[Code]
var
  DownloadPage: TDownloadWizardPage;
  ExtractPage: TOutputProgressWizardPage;
  LangPage: TWizardPage;
  LangCheckboxes: array of TCheckBox;
  { Details controls added to ExtractPage }
  DetailsMemo: TNewMemo;
  DetailsBtn: TNewButton;
  DetailsVisible: Boolean;

const
  LANG_COUNT = 5;

function GetLangCode(Index: Integer): String;
begin
  case Index of
    0: Result := 'id';
    1: Result := 'zh';
    2: Result := 'ja';
    3: Result := 'es';
    4: Result := 'ar';
  else
    Result := '';
  end;
end;

function GetLangName(Index: Integer): String;
begin
  case Index of
    0: Result := 'Indonesian';
    1: Result := 'Chinese';
    2: Result := 'Japanese';
    3: Result := 'Spanish';
    4: Result := 'Arabic';
  else
    Result := '';
  end;
end;

function GetLangLabel(Index: Integer): String;
begin
  case Index of
    0: Result := 'Indonesian (id)    ~30 MB  (required)';
    1: Result := 'Chinese (zh)       ~60 MB';
    2: Result := 'Japanese (ja)      ~50 MB';
    3: Result := 'Spanish (es)       ~40 MB';
    4: Result := 'Arabic (ar)        ~50 MB';
  else
    Result := '';
  end;
end;

procedure LogDetail(const Msg: String);
begin
  DetailsMemo.Lines.Add(Msg);
  DetailsMemo.SelStart := Length(DetailsMemo.Text);
  Log(Msg);
end;

procedure OnDetailsButtonClick(Sender: TObject);
begin
  DetailsVisible := not DetailsVisible;
  DetailsMemo.Visible := DetailsVisible;
  if DetailsVisible then
    DetailsBtn.Caption := 'Hide details'
  else
    DetailsBtn.Caption := 'Show details';
end;

function OnDownloadProgress(const Url, FileName: String; const Progress, ProgressMax: Int64): Boolean;
begin
  if ProgressMax <> 0 then
    Log(Format('Downloading %s: %d of %d bytes', [FileName, Progress, ProgressMax]));
  Result := True;
end;

procedure InitializeWizard;
var
  I: Integer;
  Lbl: TNewStaticText;
begin
  DownloadPage := CreateDownloadPage(
    SetupMessage(msgWizardPreparing),
    SetupMessage(msgPreparingDesc),
    @OnDownloadProgress);

  ExtractPage := CreateOutputProgressPage(
    'Installing',
    'Downloading and extracting LibreTranslate components...');

  { Add Show/Hide details button to ExtractPage }
  DetailsBtn := TNewButton.Create(WizardForm);
  DetailsBtn.Parent := ExtractPage.Surface;
  DetailsBtn.Top := 58;
  DetailsBtn.Left := 0;
  DetailsBtn.Width := 100;
  DetailsBtn.Height := 24;
  DetailsBtn.Caption := 'Show details';
  DetailsBtn.OnClick := @OnDetailsButtonClick;

  { Add details memo to ExtractPage (hidden by default) }
  DetailsMemo := TNewMemo.Create(WizardForm);
  DetailsMemo.Parent := ExtractPage.Surface;
  DetailsMemo.Top := 88;
  DetailsMemo.Left := 0;
  DetailsMemo.Width := ExtractPage.Surface.Width;
  DetailsMemo.Height := 160;
  DetailsMemo.ReadOnly := True;
  DetailsMemo.ScrollBars := ssVertical;
  DetailsMemo.Font.Name := 'Consolas';
  DetailsMemo.Font.Size := 8;
  DetailsMemo.Visible := False;
  DetailsVisible := False;

  { --- Language selection page --- }
  LangPage := CreateCustomPage(wpSelectTasks,
    'Language Selection',
    'Select which languages to install (English and Indonesian are always included).');

  Lbl := TNewStaticText.Create(LangPage);
  Lbl.Parent := LangPage.Surface;
  Lbl.Caption := 'Choose translation language pairs (English <-> Language):';
  Lbl.Top := 0;
  Lbl.Left := 0;
  Lbl.Width := LangPage.SurfaceWidth;

  SetArrayLength(LangCheckboxes, LANG_COUNT);
  for I := 0 to LANG_COUNT - 1 do
  begin
    LangCheckboxes[I] := TCheckBox.Create(LangPage);
    LangCheckboxes[I].Parent := LangPage.Surface;
    LangCheckboxes[I].Caption := GetLangLabel(I);
    LangCheckboxes[I].Top := 30 + I * 28;
    LangCheckboxes[I].Left := 8;
    LangCheckboxes[I].Width := LangPage.SurfaceWidth - 16;
    LangCheckboxes[I].Checked := True;
  end;
  { Indonesian (index 0) is the default target_lang — always required }
  LangCheckboxes[0].Enabled := False;
end;

function NextButtonClick(CurPageID: Integer): Boolean;
var
  I: Integer;
  AnyChecked: Boolean;
begin
  Result := True;

  if CurPageID = LangPage.ID then
  begin
    AnyChecked := False;
    for I := 0 to LANG_COUNT - 1 do
    begin
      if LangCheckboxes[I].Checked then
      begin
        AnyChecked := True;
        Break;
      end;
    end;

    if not AnyChecked then
    begin
      MsgBox('Please select at least one language.', mbError, MB_OK);
      Result := False;
    end;
  end;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  BaseUrl: String;
  ResultCode: Integer;
  DestDir: String;
  PsCmd: String;
  ZipPath: String;
  I: Integer;
  LangCode: String;
  LangList: String;
  ManifestPath: String;
  TotalSteps: Integer;
  CurrentStep: Integer;
begin
  Result := '';
  Exec('taskkill', '/F /IM {#MyAppExeName}', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Sleep(500);

  BaseUrl := '{#MyAppURL}/releases/download/v{#MyAppVersion}';
  DestDir := ExpandConstant('{app}\libretranslate');

  { Skip download/extract if LibreTranslate bundle already exists (e.g. silent update) }
  if FileExists(DestDir + '\python.exe') then begin
    Log('LibreTranslate bundle exists, skipping download.');
    exit;
  end;

  { Count selected languages for progress }
  TotalSteps := 1; { base extract }
  for I := 0 to LANG_COUNT - 1 do
    if LangCheckboxes[I].Checked then
      TotalSteps := TotalSteps + 1;

  { ====== DOWNLOAD PHASE ====== }

  DownloadPage.Clear;
  DownloadPage.Add(BaseUrl + '/libretranslate-base-windows-x64.zip', 'libretranslate-base.zip', '');

  for I := 0 to LANG_COUNT - 1 do
  begin
    if LangCheckboxes[I].Checked then
    begin
      LangCode := GetLangCode(I);
      DownloadPage.Add(
        BaseUrl + '/argos-lang-en-' + LangCode + '-windows-x64.zip',
        'argos-lang-en-' + LangCode + '.zip',
        '');
    end;
  end;

  DownloadPage.Show;
  try
    try
      DownloadPage.Download;
    except
      Result := 'Download failed: ' + GetExceptionMessage;
      exit;
    end;
  finally
    DownloadPage.Hide;
  end;

  { ====== EXTRACT PHASE ====== }

  DetailsMemo.Lines.Clear;
  CurrentStep := 0;

  ExtractPage.Show;
  try
    ForceDirectories(DestDir);

    { Extract base bundle }
    CurrentStep := CurrentStep + 1;
    ExtractPage.SetProgress(CurrentStep - 1, TotalSteps);
    ExtractPage.Msg1Label.Caption := Format('[%d / %d]  Extracting base bundle...', [CurrentStep, TotalSteps]);
    ExtractPage.Msg2Label.Caption := 'Python + LibreTranslate runtime';
    LogDetail('Extracting base bundle to ' + DestDir + '...');

    ZipPath := ExpandConstant('{tmp}\libretranslate-base.zip');
    PsCmd := Format('-NoProfile -ExecutionPolicy Bypass -Command "Expand-Archive -Path ''%s'' -DestinationPath ''%s'' -Force"', [ZipPath, DestDir]);
    if not Exec('powershell.exe', PsCmd, '', SW_HIDE, ewWaitUntilTerminated, ResultCode) then
    begin
      Result := 'Failed to extract base bundle (exit code ' + IntToStr(ResultCode) + ')';
      LogDetail('ERROR: ' + Result);
      exit;
    end;
    LogDetail('Base bundle extracted OK');

    { Extract each selected language }
    ForceDirectories(DestDir + '\argos-packages');
    for I := 0 to LANG_COUNT - 1 do
    begin
      if LangCheckboxes[I].Checked then
      begin
        LangCode := GetLangCode(I);
        CurrentStep := CurrentStep + 1;
        ExtractPage.SetProgress(CurrentStep - 1, TotalSteps);
        ExtractPage.Msg1Label.Caption := Format('[%d / %d]  Extracting %s language models...', [CurrentStep, TotalSteps, GetLangName(I)]);
        ExtractPage.Msg2Label.Caption := 'argos-lang-en-' + LangCode + '.zip';
        LogDetail('Extracting ' + GetLangName(I) + ' (' + LangCode + ') models...');

        ZipPath := ExpandConstant('{tmp}\argos-lang-en-' + LangCode + '.zip');
        PsCmd := Format('-NoProfile -ExecutionPolicy Bypass -Command "Expand-Archive -Path ''%s'' -DestinationPath ''%s'' -Force"', [ZipPath, DestDir + '\argos-packages']);
        if not Exec('powershell.exe', PsCmd, '', SW_HIDE, ewWaitUntilTerminated, ResultCode) then
        begin
          Result := 'Failed to extract ' + GetLangName(I) + ' language models.';
          LogDetail('ERROR: ' + Result);
          exit;
        end;
        LogDetail(GetLangName(I) + ' models extracted OK');
      end;
    end;

    ExtractPage.SetProgress(TotalSteps, TotalSteps);

    { Write installed-languages.txt manifest }
    LangList := 'en';
    for I := 0 to LANG_COUNT - 1 do
    begin
      if LangCheckboxes[I].Checked then
        LangList := LangList + ',' + GetLangCode(I);
    end;
    ManifestPath := DestDir + '\installed-languages.txt';
    SaveStringToFile(ManifestPath, LangList, False);
    LogDetail('');
    LogDetail('Wrote language manifest: ' + LangList);

    ExtractPage.Msg1Label.Caption := 'Done!';
    ExtractPage.Msg2Label.Caption := 'All components installed successfully.';
    LogDetail('Installation complete!');
  finally
    ExtractPage.Hide;
  end;
end;
