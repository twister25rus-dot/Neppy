import { parentPort, workerData } from "node:worker_threads";
import { ed25519 } from "@noble/curves/ed25519.js";
import { publicKeyToSolanaAddress } from "../crypto.js";
import { bytesToHex } from "./args.js";

// A single grind worker. It generates random keypairs and reports the first whose
// Solana address starts (case-insensitively) with `needle`, then exits. The main
// thread owns the deadline and terminates idle workers when one wins or time runs
// out — so the worker itself loops forever until interrupted or it finds a hit.

interface VanityWorkerData {
  needle: string;
  prefixLength: number;
}

function grind({ needle, prefixLength }: VanityWorkerData): void {
  const seed = new Uint8Array(32);
  let attempts = 0;
  for (;;) {
    // Batch between (cheap) clock-free iterations; the main thread interrupts via
    // terminate(), so we never need to poll a deadline here.
    for (let index = 0; index < 5000; index += 1) {
      attempts += 1;
      globalThis.crypto.getRandomValues(seed);
      const address = publicKeyToSolanaAddress(ed25519.getPublicKey(seed));
      if (address.slice(0, prefixLength).toLowerCase() === needle) {
        parentPort?.postMessage({
          seedHex: bytesToHex(seed),
          address,
          attempts,
        });
        return;
      }
    }
  }
}

grind(workerData as VanityWorkerData);
