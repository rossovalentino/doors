// DOTNET-A probe (P18 M1). Each step prints "DOTNET-A <n> <name> ok <detail>"
// or "... FAILED <reason>"; the last line is "DOTNET-A PASSED" or
// "DOTNET-A FAILED". Everything is deterministic except the environment
// step's runtime description, which is printed for the record.
using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Threading.Tasks;

static class Probe
{
    static int failures;

    static void Step(int n, string name, Func<string> body)
    {
        string detail;
        try
        {
            detail = body();
            Console.WriteLine($"DOTNET-A {n} {name} ok {detail}");
        }
        catch (Exception e)
        {
            failures++;
            // The whole chain: a wrapper (TaskSchedulerException,
            // TypeInitializationException) names the cause only inside.
            var chain = new List<string>();
            for (Exception x = e; x != null; x = x.InnerException)
                chain.Add($"{x.GetType().Name} (0x{x.HResult:X8}): {x.Message}");
            Console.WriteLine($"DOTNET-A {n} {name} FAILED {string.Join(" <- ", chain)}");
            string frames = e.InnerException?.StackTrace ?? e.StackTrace ?? "";
            foreach (string frame in frames.Split('\n').Take(6))
                Console.WriteLine($"DOTNET-A {n} at {frame.Trim()}");
        }
    }

    // Hot enough for tiered compilation to promote it to optimized code.
    static long Collatz(long n)
    {
        long steps = 0;
        while (n != 1)
        {
            n = (n & 1) == 0 ? n / 2 : 3 * n + 1;
            steps++;
        }
        return steps;
    }

    static int Main(string[] args)
    {
        Step(1, "hello", () => "Macrohard Doors");
        Step(2, "jit", () =>
        {
            long total = 0;
            for (int round = 0; round < 40; round++)
            {
                for (long i = 1; i < 20000; i++) total += Collatz(i);
            }
            return total.ToString();
        });
        Step(3, "generics", () =>
        {
            var map = new Dictionary<string, List<int>>();
            for (int i = 0; i < 1000; i++)
            {
                string key = "k" + (i % 17);
                if (!map.TryGetValue(key, out var list)) map[key] = list = new List<int>();
                list.Add(i * i);
            }
            var top = map.OrderBy(p => p.Key, StringComparer.Ordinal)
                .Select(p => p.Key + "=" + p.Value.Sum())
                .Take(3);
            return string.Join(",", top);
        });
        Step(4, "exceptions", () =>
        {
            int caught = 0;
            for (int i = 0; i < 50; i++)
            {
                try
                {
                    if (i % 2 == 0) throw new InvalidOperationException("even " + i);
                    int[] a = new int[1];
                    a[i] = 1;
                }
                catch (InvalidOperationException) { caught++; }
                catch (IndexOutOfRangeException) { caught += 100; }
                finally { caught += 1000; }
            }
            object o = null;
            try { o.ToString(); } catch (NullReferenceException) { caught += 7; }
            return caught.ToString();
        });
        Step(5, "threads", () =>
        {
            long sum = 0;
            var tasks = Enumerable.Range(0, 8)
                .Select(k => Task.Run(() =>
                {
                    long local = 0;
                    for (int i = 0; i < 100000; i++) local += i % (k + 2);
                    Interlocked.Add(ref sum, local);
                }))
                .ToArray();
            Task.WaitAll(tasks);
            using var gate = new ManualResetEventSlim(false);
            var thread = new Thread(() => gate.Set());
            thread.Start();
            gate.Wait(5000);
            thread.Join();
            return sum.ToString();
        });
        Step(6, "gc", () =>
        {
            var keep = new List<byte[]>();
            for (int i = 0; i < 2000; i++)
            {
                var b = new byte[8192];
                b[i % 8192] = (byte)i;
                if (i % 100 == 0) keep.Add(b);
            }
            GC.Collect();
            GC.WaitForPendingFinalizers();
            return keep.Count + " " + (GC.CollectionCount(0) > 0);
        });
        // Guest only: the owner's PC is read-only.
        if (args.Length > 0 && args[0] == "guest") Step(7, "files", () =>
        {
            string dir = Path.Combine(Path.GetTempPath(), "dotnet-a");
            Directory.CreateDirectory(dir);
            string file = Path.Combine(dir, "probe.txt");
            File.WriteAllText(file, "line one\nline two\n");
            File.AppendAllLines(file, new[] { "third" });
            string[] lines = File.ReadAllLines(file);
            long length = new FileInfo(file).Length;
            var listed = Directory.GetFiles(dir).Select(Path.GetFileName).ToArray();
            File.Delete(file);
            Directory.Delete(dir);
            return $"{lines.Length} {length} {string.Join("|", listed)} {File.Exists(file)}";
        });
        Step(8, "text", () =>
        {
            var sb = new StringBuilder();
            sb.Append(Math.Sqrt(2).ToString("F6", System.Globalization.CultureInfo.InvariantCulture));
            sb.Append(' ').Append(Convert.ToBase64String(Encoding.UTF8.GetBytes("Doors ✓")));
            sb.Append(' ').Append(string.Format("{0:X8}", 0xC0FFEE));
            return sb.ToString();
        });
        Step(9, "environment", () =>
            $"{Environment.Is64BitProcess} {Environment.ProcessorCount > 0} {RuntimeInformation.OSArchitecture} [{RuntimeInformation.FrameworkDescription}]");
        Console.WriteLine(failures == 0 ? "DOTNET-A PASSED" : "DOTNET-A FAILED");
        return failures;
    }
}
