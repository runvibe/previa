import type { LoadRunConfig, LoadTestMetrics, LoadTestState } from "@/types/load-test";
import { generateUUID } from "@/lib/uuid";

export interface LoadTestRunRecord {
  id: string;
  projectId: string;
  pipelineIndex: number;
  pipelineName: string;
  config: LoadRunConfig;
  metrics: LoadTestMetrics;
  state: LoadTestState;
  timestamp: string;
  executionId?: string;
}

const DB_NAME = "previa-loadtests-v1";
const DB_VERSION = 1;
const STORE_NAME = "runs";

function openDB(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(DB_NAME, DB_VERSION);
    req.onupgradeneeded = () => {
      const db = req.result;
      if (!db.objectStoreNames.contains(STORE_NAME)) {
        const store = db.createObjectStore(STORE_NAME, { keyPath: "id" });
        store.createIndex("project_pipeline", ["projectId", "pipelineIndex"], { unique: false });
        store.createIndex("projectId", "projectId", { unique: false });
      }
    };
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

export async function saveLoadTestRun(run: Omit<LoadTestRunRecord, "id">): Promise<string> {
  const db = await openDB();
  const id = generateUUID();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE_NAME, "readwrite");
    const store = tx.objectStore(STORE_NAME);
    const req = store.add({ ...run, id });
    req.onsuccess = () => resolve(id);
    req.onerror = () => reject(req.error);
  });
}

export async function getLoadTestRuns(projectId: string, pipelineIndex: number): Promise<LoadTestRunRecord[]> {
  const db = await openDB();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE_NAME, "readonly");
    const store = tx.objectStore(STORE_NAME);
    const index = store.index("project_pipeline");
    const req = index.getAll(IDBKeyRange.only([projectId, pipelineIndex]));
    req.onsuccess = () => {
      const runs = (req.result as LoadTestRunRecord[]).sort(
        (a, b) => new Date(b.timestamp).getTime() - new Date(a.timestamp).getTime()
      );
      resolve(runs);
    };
    req.onerror = () => reject(req.error);
  });
}

export async function getAllLoadTestRunsForProject(projectId: string): Promise<LoadTestRunRecord[]> {
  const db = await openDB();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE_NAME, "readonly");
    const store = tx.objectStore(STORE_NAME);
    const index = store.index("projectId");
    const req = index.getAll(IDBKeyRange.only(projectId));
    req.onsuccess = () => {
      const runs = (req.result as LoadTestRunRecord[]).sort(
        (a, b) => new Date(a.timestamp).getTime() - new Date(b.timestamp).getTime()
      );
      resolve(runs);
    };
    req.onerror = () => reject(req.error);
  });
}

export async function deleteLoadTestRunsForPipeline(projectId: string, pipelineIndex: number): Promise<void> {
  const db = await openDB();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE_NAME, "readwrite");
    const store = tx.objectStore(STORE_NAME);
    const index = store.index("project_pipeline");
    const req = index.openCursor(IDBKeyRange.only([projectId, pipelineIndex]));
    req.onsuccess = () => {
      const cursor = req.result;
      if (cursor) {
        cursor.delete();
        cursor.continue();
      } else {
        resolve();
      }
    };
    req.onerror = () => reject(req.error);
  });
}
