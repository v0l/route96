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

  useEffect(() => {
    if (userInfo?.quota && userInfo.quota > 0) {
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
    return <div className="text-red-400 text-xs">Payment not available: {error}</div>;
  }

  if (!paymentInfo) {
    return <div className="text-neutral-500 text-xs">Loading payment info...</div>;
  }

  const totalCostBTC = paymentInfo.cost.amount * gigabytes * months;
  const totalCostSats = Math.round(totalCostBTC * 100000000);

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
    <div className="bg-neutral-900 border border-neutral-800 rounded-sm p-3">
      <h3 className="text-sm font-medium mb-3 text-white">Top Up</h3>
      <div className="space-y-3">
        <div className="text-center">
          <div className="text-lg font-medium text-white">
            {gigabytes} {formatStorageUnit(paymentInfo.unit)} x {months}mo
          </div>
          <div className="text-sm text-neutral-400">
            {totalCostSats.toLocaleString()} sats
          </div>
        </div>

        <div className="grid grid-cols-2 gap-2">
          <div>
            <label className="block text-xs text-neutral-500 mb-1">
              {formatStorageUnit(paymentInfo.unit)}
            </label>
            <input
              type="number"
              min="1"
              step="1"
              value={gigabytes}
              onChange={(e) => setGigabytes(parseInt(e.target.value) || 1)}
              className="w-full h-8 rounded-sm border border-neutral-800 bg-neutral-950 px-2 text-center text-sm text-white"
            />
          </div>

          <div>
            <label className="block text-xs text-neutral-500 mb-1">
              Months
            </label>
            <input
              type="number"
              min="1"
              step="1"
              value={months}
              onChange={(e) => setMonths(parseInt(e.target.value) || 1)}
              className="w-full h-8 rounded-sm border border-neutral-800 bg-neutral-950 px-2 text-center text-sm text-white"
            />
          </div>
        </div>

        <Button
          onClick={requestPayment}
          disabled={loading || gigabytes <= 0 || months <= 0}
          className="w-full"
          size="sm"
        >
          {loading ? "..." : "Generate Invoice"}
        </Button>

        {error && <div className="text-red-400 text-xs">{error}</div>}

        {paymentRequest && (
          <div className="bg-neutral-950 border border-neutral-800 rounded-sm p-2">
            <div className="text-xs text-neutral-500 mb-1">Lightning Invoice:</div>
            <code className="text-xs text-neutral-300 break-all block">
              {paymentRequest}
            </code>
            <div className="text-xs text-neutral-600 mt-1">
              Copy to your Lightning wallet
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
