import { useState, useEffect } from "react";
import Button from "./button";
import {
  PaymentInfo,
  PaymentRequest,
  Route96,
  AdminSelf,
} from "../upload/admin";

interface PaymentFlowProps {
  route96: Route96;
  onPaymentRequested?: (paymentRequest: string) => void;
  userInfo?: AdminSelf;
}

export default function PaymentFlow({
  route96,
  onPaymentRequested,
  userInfo,
}: PaymentFlowProps) {
  const [paymentInfo, setPaymentInfo] = useState<PaymentInfo | null>(null);
  const [gigabytes, setGigabytes] = useState<number>(1);
  const [months, setMonths] = useState<number>(1);
  const [paymentRequest, setPaymentRequest] = useState<string>("");
  const [error, setError] = useState<string>("");
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (paymentInfo === null) {
      loadPaymentInfo();
    }
  }, [paymentInfo]);

  // Set default gigabytes to user's current quota
  useEffect(() => {
    if (userInfo?.quota && userInfo.quota > 0) {
      // Convert from bytes to GB using 1024^3 (MiB)
      const currentQuotaGB = Math.round(userInfo.quota / (1024 * 1024 * 1024));
      if (currentQuotaGB > 0) {
        setGigabytes(currentQuotaGB);
      }
    }
  }, [userInfo]);

  async function loadPaymentInfo() {
    try {
      const info = await route96.getPaymentInfo();
      setPaymentInfo(info);
    } catch (e) {
      if (e instanceof Error) {
        setError(e.message);
      } else {
        setError("Failed to load payment info");
      }
    }
  }

  async function requestPayment() {
    if (!paymentInfo) return;

    setLoading(true);
    setError("");

    try {
      const request: PaymentRequest = { units: gigabytes, quantity: months };
      const response = await route96.requestPayment(request);
      setPaymentRequest(response.pr);
      onPaymentRequested?.(response.pr);
    } catch (e) {
      if (e instanceof Error) {
        setError(e.message);
      } else {
        setError("Failed to request payment");
      }
    } finally {
      setLoading(false);
    }
  }

  if (error && !paymentInfo) {
    return <div className="text-red-400">Payment not available: {error}</div>;
  }

  if (!paymentInfo) {
    return <div className="text-neutral-400">Loading payment info...</div>;
  }

  const totalCostBTC = paymentInfo.cost.amount * gigabytes * months;
  const totalCostSats = Math.round(totalCostBTC * 100000000); // Convert BTC to sats

  function formatStorageUnit(unit: string): string {
    if (
      unit.toLowerCase().includes("gbspace") ||
      unit.toLowerCase().includes("gb")
    ) {
      return "GB";
    }
    return unit;
  }

  return (
    <div className="bg-neutral-800 border border-neutral-700 rounded-lg shadow-sm">
      <div className="p-6">
        <h3 className="text-lg font-semibold mb-6 text-neutral-100">Top Up Account</h3>
        <div className="space-y-6">
          <div className="text-center">
            <div className="text-2xl font-bold mb-2 text-neutral-100">
              {gigabytes} {formatStorageUnit(paymentInfo.unit)} for {months} month
              {months > 1 ? "s" : ""}
            </div>
            <div className="text-lg text-neutral-300 font-semibold">
              {totalCostSats.toLocaleString()} sats
            </div>
          </div>

          <div className="grid grid-cols-2 gap-4">
            <div>
              <label className="block text-sm font-medium mb-2 text-neutral-300">
                Storage ({formatStorageUnit(paymentInfo.unit)})
              </label>
              <input
                type="number"
                min="1"
                step="1"
                value={gigabytes}
                onChange={(e) => setGigabytes(parseInt(e.target.value) || 1)}
                className="flex h-10 w-full rounded-md border border-neutral-600 bg-neutral-700 px-3 py-2 text-center text-lg text-neutral-100 ring-offset-neutral-800 placeholder:text-neutral-400 focus:outline-none focus:ring-2 focus:ring-neutral-500 focus:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50"
              />
            </div>

            <div>
              <label className="block text-sm font-medium mb-2 text-neutral-300">
                Duration (months)
              </label>
              <input
                type="number"
                min="1"
                step="1"
                value={months}
                onChange={(e) => setMonths(parseInt(e.target.value) || 1)}
                className="flex h-10 w-full rounded-md border border-neutral-600 bg-neutral-700 px-3 py-2 text-center text-lg text-neutral-100 ring-offset-neutral-800 placeholder:text-neutral-400 focus:outline-none focus:ring-2 focus:ring-neutral-500 focus:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50"
              />
            </div>
          </div>

          <Button
            onClick={requestPayment}
            disabled={loading || gigabytes <= 0 || months <= 0}
            className="w-full"
          >
            {loading ? "Processing..." : "Generate Payment Request"}
          </Button>

          {error && <div className="text-red-400 text-sm">{error}</div>}

          {paymentRequest && (
            <div className="bg-neutral-700 border border-neutral-600 rounded-lg">
              <div className="p-4">
                <div className="text-sm font-medium mb-2 text-neutral-200">Lightning Invoice:</div>
                <div className="font-mono text-xs break-all bg-neutral-800 text-neutral-200 p-2 rounded">
                  {paymentRequest}
                </div>
                <div className="text-xs text-neutral-400 mt-2">
                  Copy this invoice to your Lightning wallet to complete payment
                </div>
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
