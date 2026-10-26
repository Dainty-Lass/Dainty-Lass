@echo off
setlocal
rem The SDK is each host's own Dumper-7 dump of the 1.4.4 client, with sdk-fixups.patch applied (README.md).
if not exist "%~dp0SDK.hpp" (
  echo SDK.hpp was not found: dump the 1.4.4 client with Dumper-7 and copy its CppSDK here (README.md^).
  exit /b 2
)
findstr /c:"rel-1.4.4" "%~dp0SDK.hpp" >nul || (
  echo SDK.hpp is not a dump of Dauntless 1.4.4.
  exit /b 2
)
findstr /c:"NetworkNotify;" "%~dp0SDK\Engine_classes.hpp" >nul || (
  echo sdk-fixups.patch is not applied to the SDK (README.md^).
  exit /b 2
)
set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
if not exist "%VSWHERE%" (
  echo Visual Studio Installer vswhere.exe was not found.
  exit /b 3
)
for /f "usebackq delims=" %%I in (`"%VSWHERE%" -latest -products * -requires Microsoft.Component.MSBuild -find MSBuild\**\Bin\MSBuild.exe`) do set "MSBUILD=%%I"
if not defined MSBUILD (
  echo MSBuild with C++ tools was not found.
  exit /b 3
)
"%MSBUILD%" "%~dp0UndauntedInternalServer.sln" /m /p:Configuration=Release /p:Platform=x64 /nologo /verbosity:minimal
exit /b %errorlevel%
