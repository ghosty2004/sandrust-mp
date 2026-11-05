// Headless Ghidra post-script for the samp.dll feature inventory (tools/cov/samp-inventory.sh).
// Writes three TSV files next to the given prefix:
//   PREFIX.functions.tsv  entry  size  name  thunk  entry_byte  callers  callees
//   PREFIX.edges.tsv      from_function  to_function  call_site            (direct calls/jumps;
//                         to = EXT:Library::Name for imports, also through thunks)
//   PREFIX.refs.tsv       function  ref_site  kind  target  text
//       kind = str (a defined string; text = its value, tabs/newlines escaped)
//            | ptr (a data pointer to a function entry: vtables, callback tables;
//                   ref_site = the data address holding the pointer, function = "-")
//            | data (other data reference from code; text empty)
// Generated data only (target/ is gitignored); describes structure, not code.
// @category COV
import java.io.PrintWriter;
import java.util.HashSet;
import java.util.Set;

import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.listing.Data;
import ghidra.program.model.listing.Function;
import ghidra.program.model.listing.FunctionIterator;
import ghidra.program.model.listing.FunctionManager;
import ghidra.program.model.listing.Instruction;
import ghidra.program.model.listing.InstructionIterator;
import ghidra.program.model.listing.Listing;
import ghidra.program.model.mem.Memory;
import ghidra.program.model.symbol.RefType;
import ghidra.program.model.symbol.Reference;
import ghidra.program.model.symbol.ReferenceManager;

public class ExportGraph extends GhidraScript {
    private static String esc(String s) {
        if (s == null) {
            return "";
        }
        StringBuilder b = new StringBuilder();
        for (char c : s.toCharArray()) {
            if (c == '\t') b.append("\\t");
            else if (c == '\n') b.append("\\n");
            else if (c == '\r') b.append("\\r");
            else if (c < 0x20) b.append(String.format("\\x%02x", (int) c));
            else b.append(c);
        }
        return b.length() > 300 ? b.substring(0, 300) : b.toString();
    }

    @Override
    public void run() throws Exception {
        String[] args = getScriptArgs();
        if (args.length < 1) {
            throw new IllegalArgumentException("usage: ExportGraph.java OUT_PREFIX");
        }
        String pre = args[0];
        FunctionManager fm = currentProgram.getFunctionManager();
        ReferenceManager rm = currentProgram.getReferenceManager();
        Listing listing = currentProgram.getListing();
        Memory mem = currentProgram.getMemory();
        int n = 0;
        try (PrintWriter wf = new PrintWriter(pre + ".functions.tsv", "UTF-8");
                PrintWriter we = new PrintWriter(pre + ".edges.tsv", "UTF-8");
                PrintWriter wr = new PrintWriter(pre + ".refs.tsv", "UTF-8")) {
            wf.println("entry\tsize\tname\tthunk\tentry_byte\tcallers\tcallees");
            we.println("from\tto\tsite");
            wr.println("function\tsite\tkind\ttarget\ttext");
            FunctionIterator it = fm.getFunctions(true);
            while (it.hasNext()) {
                Function f = it.next();
                Address e = f.getEntryPoint();
                int b = -1;
                try {
                    b = mem.getByte(e) & 0xff;
                } catch (Exception ex) {
                    // uninitialised / external
                }
                Set<Function> callers = f.getCallingFunctions(monitor);
                Set<Function> callees = f.getCalledFunctions(monitor);
                wf.printf("0x%s\t%d\t%s\t%d\t%s\t%d\t%d%n", e, f.getBody().getNumAddresses(), f.getName(true),
                        f.isThunk() ? 1 : 0, b < 0 ? "-" : String.format("%02x", b), callers.size(), callees.size());
                Set<String> seen = new HashSet<>();
                InstructionIterator ii = listing.getInstructions(f.getBody(), true);
                while (ii.hasNext()) {
                    Instruction ins = ii.next();
                    for (Reference r : ins.getReferencesFrom()) {
                        RefType t = r.getReferenceType();
                        Address to = r.getToAddress();
                        if (t.isCall() || t.isJump()) {
                            Function g = fm.getFunctionAt(to);
                            if (g == null && t.isJump()) {
                                continue; // intra-function branch
                            }
                            if (g != null && !g.equals(f)) {
                                if (g.isExternal() || g.isThunk() && g.getThunkedFunction(true) != null
                                        && g.getThunkedFunction(true).isExternal()) {
                                    Function x = g.isExternal() ? g : g.getThunkedFunction(true);
                                    we.printf("0x%s\tEXT:%s\t0x%s%n", e, x.getName(true), ins.getAddress());
                                } else {
                                    we.printf("0x%s\t0x%s\t0x%s%n", e, g.getEntryPoint(), ins.getAddress());
                                }
                            }
                            continue;
                        }
                        if (!to.isMemoryAddress()) {
                            continue;
                        }
                        Data d = listing.getDataContaining(to);
                        String key = to.toString();
                        if (!seen.add(key)) {
                            continue;
                        }
                        if (d != null && d.hasStringValue()) {
                            Object v = d.getValue();
                            wr.printf("0x%s\t0x%s\tstr\t0x%s\t%s%n", e, ins.getAddress(), to, esc(String.valueOf(v)));
                        } else if (fm.getFunctionAt(to) != null) {
                            wr.printf("0x%s\t0x%s\tfnref\t0x%s\t%n", e, ins.getAddress(), to);
                        } else {
                            wr.printf("0x%s\t0x%s\tdata\t0x%s\t%n", e, ins.getAddress(), to);
                        }
                    }
                }
                // Data pointers to this entry (vtables, callback tables, registration tables).
                for (Reference r : rm.getReferencesTo(e)) {
                    Address from = r.getFromAddress();
                    if (listing.getInstructionAt(from) != null) {
                        continue;
                    }
                    wr.printf("-\t0x%s\tptr\t0x%s\t%n", from, e);
                }
                n++;
            }
        }
        println("ExportGraph: " + n + " functions -> " + pre + ".*.tsv");
    }
}
