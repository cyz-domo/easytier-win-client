Get-ChildItem '\\.\pipe\' | Where-Object Name -like '*EasyTier*' | Select-Object -ExpandProperty Name
Write-Output '--- service query ---'
sc.exe query EasyTierService
