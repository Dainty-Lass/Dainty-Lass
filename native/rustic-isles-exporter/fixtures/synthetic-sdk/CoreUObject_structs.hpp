// Synthetic SDK for the catalog exporter's tests: made up, never generated from a game.

#pragma once

// ScriptStruct Synthetic.SoftObjectPath
// 0x0018 (0x0018 - 0x0000)
struct FSoftObjectPath
{
public:
	class FName                                   AssetPathName;                                     // 0x0000(0x0008)(Edit, NativeAccessSpecifierPublic)
	class FString                                 SubPathString;                                     // 0x0008(0x0010)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.SoftClassPath
// 0x0000 (0x0018 - 0x0018)
struct FSoftClassPath : public FSoftObjectPath
{
};
