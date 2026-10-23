// Static readers only. There are intentionally NO install, configure,
// package-session, registry, or process-launch APIs in this helper.
using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Globalization;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.RegularExpressions;

namespace NetSpeedPackaging
{
    public sealed class MsiTable
    {
        public string Name { get; set; }
        public string[] Columns { get; set; }
        public List<Dictionary<string, string>> Rows { get; set; }
    }

    public sealed class CabinetMember
    {
        public string Name { get; set; }
        public uint Size { get; set; }
        public ushort Attributes { get; set; }
    }

    public sealed class ReadOnlyMsi : IDisposable
    {
        private uint database;
        private const uint MoreData = 234;
        private const uint NoMoreItems = 259;

        public ReadOnlyMsi(string path)
        {
            // MSIDBOPEN_READONLY is the null pointer, NOT an installation session.
            Check(Native.MsiOpenDatabaseW(Path.GetFullPath(path), IntPtr.Zero, out database), "MsiOpenDatabase(read-only)");
        }

        public MsiTable ReadTable(string table, string[] columns)
        {
            EnsureOpen();
            ValidateIdentifier(table);
            if (columns == null || columns.Length == 0) throw new ArgumentException("At least one column is required.");
            var quoted = new string[columns.Length];
            for (int i = 0; i < columns.Length; i++)
            {
                ValidateIdentifier(columns[i]);
                quoted[i] = "`" + columns[i] + "`";
            }
            string sql = "SELECT " + String.Join(", ", quoted) + " FROM `" + table + "`";
            uint view = OpenSelect(sql);
            try
            {
                var rows = new List<Dictionary<string, string>>();
                while (true)
                {
                    uint record;
                    uint result = Native.MsiViewFetch(view, out record);
                    if (result == NoMoreItems) break;
                    Check(result, "MsiViewFetch");
                    try
                    {
                        var row = new Dictionary<string, string>(StringComparer.Ordinal);
                        for (int i = 0; i < columns.Length; i++) row[columns[i]] = ReadString(record, (uint)i + 1);
                        rows.Add(row);
                    }
                    finally { Native.MsiCloseHandle(record); }
                }
                return new MsiTable { Name = table, Columns = columns, Rows = rows };
            }
            finally { Native.MsiCloseHandle(view); }
        }

        public Dictionary<string, string> ReadSummary()
        {
            EnsureOpen();
            uint summary;
            Check(Native.MsiGetSummaryInformationW(database, null, 0, out summary), "MsiGetSummaryInformation(read-only)");
            try
            {
                var values = new Dictionary<string, string>(StringComparer.Ordinal);
                for (uint property = 1; property <= 19; property++)
                {
                    uint type;
                    int number;
                    Native.FileTime time;
                    uint capacity = 1024;
                    var text = new StringBuilder((int)capacity);
                    uint result = Native.MsiSummaryInfoGetPropertyW(summary, property, out type, out number, out time, text, ref capacity);
                    if (result == MoreData)
                    {
                        capacity++;
                        text = new StringBuilder(checked((int)capacity));
                        result = Native.MsiSummaryInfoGetPropertyW(summary, property, out type, out number, out time, text, ref capacity);
                    }
                    Check(result, "MsiSummaryInfoGetProperty");
                    string value;
                    if (type == 2 || type == 3) value = number.ToString(CultureInfo.InvariantCulture); // VT_I2/VT_I4
                    else if (type == 64)
                    {
                        long timestamp = ((long)time.High << 32) | time.Low;
                        value = timestamp == 0 ? "" : DateTime.FromFileTimeUtc(timestamp).ToString("o", CultureInfo.InvariantCulture);
                    }
                    else value = text.ToString();
                    values[property.ToString(CultureInfo.InvariantCulture)] = value;
                }
                return values;
            }
            finally { Native.MsiCloseHandle(summary); }
        }

        public void ExportStream(string name, string outputPath)
        {
            ExportBinary("_Streams", "Name", name, "Data", outputPath);
        }

        public void ExportBinary(string table, string keyColumn, string key, string dataColumn, string outputPath)
        {
            EnsureOpen();
            if (table != "_Streams" && table != "Icon" && table != "Binary") throw new ArgumentException("Only binary stream tables are allowed.");
            ValidateIdentifier(keyColumn);
            ValidateIdentifier(dataColumn);
            string sql = "SELECT `" + dataColumn + "` FROM `" + table + "` WHERE `" + keyColumn + "` = '" + key.Replace("'", "''") + "'";
            uint view = OpenSelect(sql);
            try
            {
                uint record;
                Check(Native.MsiViewFetch(view, out record), "Find embedded stream: " + key);
                try
                {
                    // CreateNew prevents an export from overwriting another file.
                    using (var output = new FileStream(outputPath, FileMode.CreateNew, FileAccess.Write, FileShare.None))
                    {
                        var buffer = new byte[65536];
                        while (true)
                        {
                            uint count = (uint)buffer.Length;
                            Check(Native.MsiRecordReadStream(record, 1, buffer, ref count), "MsiRecordReadStream");
                            if (count == 0) break;
                            output.Write(buffer, 0, checked((int)count));
                        }
                    }
                }
                finally { Native.MsiCloseHandle(record); }
            }
            finally { Native.MsiCloseHandle(view); }
        }

        public static CabinetMember[] InspectCabinet(string path)
        {
            var members = new List<CabinetMember>();
            Exception failure = null;
            Native.CabinetCallback callback = delegate(IntPtr context, uint notification, UIntPtr parameter1, UIntPtr parameter2)
            {
                try
                {
                    if (notification == 0x11) // SPFILENOTIFY_FILEINCABINET
                    {
                        var info = Marshal.PtrToStructure<Native.FileInCabinetInfo>(new IntPtr(unchecked((long)parameter1.ToUInt64())));
                        string name = Marshal.PtrToStringUni(info.NameInCabinet);
                        if (String.IsNullOrEmpty(name) || name == "." || name == ".." ||
                            name.IndexOfAny(new[] { '\\', '/', ':' }) >= 0 || name != Path.GetFileName(name))
                            throw new InvalidDataException("Unsafe cabinet member name: " + name);
                        members.Add(new CabinetMember { Name = name, Size = info.FileSize, Attributes = info.DosAttributes });
                        return 2; // FILEOP_SKIP: inspect ONLY; absolutely no file extraction here.
                    }
                    if (notification == 0x12) throw new InvalidDataException("Multi-part cabinets are not supported."); // NEEDNEWCABINET
                    return 0;
                }
                catch (Exception error)
                {
                    failure = error;
                    // FILEINCABINET expects FILEOP_ABORT (0), whereas other
                    // notifications return a Win32 error code. Never throw
                    // across a callback or allow an unsafe member to extract.
                    return notification == 0x11 ? 0u : 1223u;
                }
            };
            bool success = Native.SetupIterateCabinetW(Path.GetFullPath(path), 0, callback, IntPtr.Zero);
            int nativeError = Marshal.GetLastWin32Error();
            GC.KeepAlive(callback);
            if (failure != null) throw failure;
            if (!success) throw new Win32Exception(nativeError, "Static cabinet inspection failed.");
            if (members.Count == 0) throw new InvalidDataException("Embedded cabinet is empty.");
            return members.ToArray();
        }

        public void Dispose()
        {
            if (database != 0) { Native.MsiCloseHandle(database); database = 0; }
        }

        private void EnsureOpen()
        {
            if (database == 0) throw new ObjectDisposedException("ReadOnlyMsi");
        }

        private uint OpenSelect(string sql)
        {
            uint view;
            Check(Native.MsiDatabaseOpenViewW(database, sql, out view), "MsiDatabaseOpenView(SELECT)");
            uint result = Native.MsiViewExecute(view, 0);
            if (result != 0) { Native.MsiCloseHandle(view); Check(result, "MsiViewExecute(SELECT)"); }
            return view;
        }

        private static string ReadString(uint record, uint field)
        {
            if (Native.MsiRecordIsNull(record, field)) return null;
            uint length = 0;
            uint result = Native.MsiRecordGetStringW(record, field, null, ref length);
            if (result != 0 && result != MoreData) Check(result, "Measure MSI string");
            if (length == 0) return "";
            length++;
            var buffer = new StringBuilder(checked((int)length));
            Check(Native.MsiRecordGetStringW(record, field, buffer, ref length), "Read MSI string");
            return buffer.ToString();
        }

        private static void ValidateIdentifier(string name)
        {
            if (name == null || !Regex.IsMatch(name, "^[A-Za-z_][A-Za-z0-9_]*$")) throw new ArgumentException("Unsafe MSI identifier.");
        }

        private static void Check(uint result, string operation)
        {
            if (result != 0) throw new Win32Exception(checked((int)result), operation);
        }

        private static class Native
        {
            [StructLayout(LayoutKind.Sequential)]
            internal struct FileTime { public uint Low; public uint High; }

            [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
            internal struct FileInCabinetInfo
            {
                public IntPtr NameInCabinet;
                public uint FileSize;
                public uint Win32Error;
                public ushort DosDate;
                public ushort DosTime;
                public ushort DosAttributes;
                [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 260)] public string FullTargetName;
            }

            [UnmanagedFunctionPointer(CallingConvention.Winapi)]
            internal delegate uint CabinetCallback(IntPtr context, uint notification, UIntPtr parameter1, UIntPtr parameter2);

            [DllImport("msi.dll", CharSet = CharSet.Unicode, ExactSpelling = true)]
            internal static extern uint MsiOpenDatabaseW(string path, IntPtr persistence, out uint database);
            [DllImport("msi.dll", CharSet = CharSet.Unicode, ExactSpelling = true)]
            internal static extern uint MsiDatabaseOpenViewW(uint database, string sql, out uint view);
            [DllImport("msi.dll", ExactSpelling = true)]
            internal static extern uint MsiViewExecute(uint view, uint record);
            [DllImport("msi.dll", ExactSpelling = true)]
            internal static extern uint MsiViewFetch(uint view, out uint record);
            [DllImport("msi.dll", ExactSpelling = true)]
            [return: MarshalAs(UnmanagedType.Bool)]
            internal static extern bool MsiRecordIsNull(uint record, uint field);
            [DllImport("msi.dll", CharSet = CharSet.Unicode, ExactSpelling = true)]
            internal static extern uint MsiRecordGetStringW(uint record, uint field, StringBuilder value, ref uint length);
            [DllImport("msi.dll", ExactSpelling = true)]
            internal static extern uint MsiRecordReadStream(uint record, uint field, [Out] byte[] value, ref uint length);
            [DllImport("msi.dll", CharSet = CharSet.Unicode, ExactSpelling = true)]
            internal static extern uint MsiGetSummaryInformationW(uint database, string path, uint updateCount, out uint summary);
            [DllImport("msi.dll", CharSet = CharSet.Unicode, ExactSpelling = true)]
            internal static extern uint MsiSummaryInfoGetPropertyW(uint summary, uint property, out uint type, out int number, out FileTime time, StringBuilder text, ref uint length);
            [DllImport("msi.dll", ExactSpelling = true)]
            internal static extern uint MsiCloseHandle(uint handle);
            [DllImport("setupapi.dll", CharSet = CharSet.Unicode, ExactSpelling = true, SetLastError = true)]
            [return: MarshalAs(UnmanagedType.Bool)]
            internal static extern bool SetupIterateCabinetW(string path, uint reserved, CabinetCallback callback, IntPtr context);
        }
    }
}
