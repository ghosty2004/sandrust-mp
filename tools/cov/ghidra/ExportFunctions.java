// Headless Ghidra post-script for the COV-1 function inventory (tools/cov/inventory.sh).
// Writes one TSV line per function of the current program:
//   entry  size  name  thunk  data_refs
// size = number of addresses in the function body; data_refs = addresses (comma separated)
// of references that READ or point at the first 8 bytes of the entry as data (not calls or
// jumps), which the tracer must not arm (protection stubs read code bytes as data).
// @category COV
import java.io.PrintWriter;
import java.util.ArrayList;
import java.util.List;

import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.listing.Function;
import ghidra.program.model.listing.FunctionIterator;
import ghidra.program.model.symbol.RefType;
import ghidra.program.model.symbol.Reference;
import ghidra.program.model.symbol.ReferenceManager;

public class ExportFunctions extends GhidraScript {
    @Override
    public void run() throws Exception {
        String[] args = getScriptArgs();
        if (args.length < 1) {
            throw new IllegalArgumentException("usage: ExportFunctions.java OUT.tsv");
        }
        ReferenceManager rm = currentProgram.getReferenceManager();
        FunctionIterator it = currentProgram.getFunctionManager().getFunctions(true);
        int n = 0;
        try (PrintWriter w = new PrintWriter(args[0], "UTF-8")) {
            w.println("entry\tsize\tname\tthunk\tdata_refs");
            while (it.hasNext()) {
                Function f = it.next();
                Address e = f.getEntryPoint();
                List<String> refs = new ArrayList<>();
                for (int i = 0; i < 8; i++) {
                    Address a = e.add(i);
                    for (Reference r : rm.getReferencesTo(a)) {
                        RefType t = r.getReferenceType();
                        if (t.isFlow() || t.isCall() || t.isJump()) {
                            continue;
                        }
                        // Plain pointers to the entry itself (vtables, callback tables) are fine.
                        if (i == 0 && !t.isRead() && !t.isWrite()) {
                            continue;
                        }
                        refs.add(r.getFromAddress() + "+" + i + ":" + t.getName());
                    }
                }
                w.printf("0x%s\t%d\t%s\t%d\t%s%n", e.toString(), f.getBody().getNumAddresses(), f.getName(true),
                        f.isThunk() ? 1 : 0, String.join(",", refs));
                n++;
            }
        }
        println("ExportFunctions: " + n + " functions -> " + args[0]);
    }
}
