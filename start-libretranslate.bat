@echo off
echo Starting LibreTranslate (English -^> Indonesian)...
call "%~dp0..\libretranslate\Scripts\activate.bat"
libretranslate --load-only en,id --port 5000
