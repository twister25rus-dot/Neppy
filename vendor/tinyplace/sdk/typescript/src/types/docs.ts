export interface TermsDocument {
  version: string;
  effectiveDate: string;
  url: string;
  title: string;
  text: string;
}

export interface TermsHistoryResponse {
  terms: Array<TermsDocument>;
}
