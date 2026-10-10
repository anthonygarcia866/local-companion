; Uninstall hooks for the NSIS installer.
;
; The app stages glim-hook.exe into %LOCALAPPDATA%\Glim\bin at launch, so the
; installer never recorded it and the default uninstaller leaves it behind. The
; inbox and the log live in the same place and are ours too.
;
; Claude Code's own settings.json is deliberately NOT touched here: it belongs to
; the user, it may contain hooks from other tools, and rewriting somebody's
; config from an uninstaller with no diff and no consent is exactly what the rest
; of this app goes out of its way not to do. A relay that is gone exits 0 without
; printing anything. A leftover entry whose relay file is gone makes Claude Code
; report a failed hook command (it carries on regardless): remove Glim's hooks
; in Settings before uninstalling to avoid that.

!macro NSIS_HOOK_PREUNINSTALL
  RMDir /r "$LOCALAPPDATA\Glim\bin"
  RMDir /r "$LOCALAPPDATA\Glim\inbox"
  Delete "$LOCALAPPDATA\Glim\glim.log"
  ; Left by a build from before the rename that never ran the migration.
  RMDir /r "$LOCALAPPDATA\Coucou\bin"
  RMDir /r "$LOCALAPPDATA\Coucou\inbox"
  Delete "$LOCALAPPDATA\Coucou\coucou.log"
!macroend
