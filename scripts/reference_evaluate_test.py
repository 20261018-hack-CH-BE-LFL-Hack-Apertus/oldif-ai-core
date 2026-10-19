"""Offline adversarial checks for the reference scorer, without provider access."""
import unittest
from reference_evaluate import score

class ReferenceScoreTests(unittest.TestCase):
    def setUp(self):
        self.source = {"id":"record", "text":"Children should learn reading.", "sha256":"original-hash", "text_sha256":"text-hash"}
        self.quote = self.source["text"]
        self.citation = {"source_id":"record", "quote":self.quote, "sha256":"original-hash", "text_sha256":"text-hash"}
        self.result = {"answer":"Unreviewed evidence.\n\n“"+self.quote+"”", "citations":[self.citation], "evidence_only":True, "human_review_required":True}
    def evaluate(self, result=None, negative=False):
        return score(result or self.result,self.source,self.quote,negative)
    def test_intended_subject_is_not_observed_outcome(self):
        self.assertTrue(self.evaluate()["case_pass"])
        self.assertFalse(self.evaluate(negative=True)["case_pass"])
    def test_unsupported_prose_cannot_be_redeemed_by_exact_quote(self):
        self.result["answer"] += " All children became literate."
        self.result["verification"] = {"verdict":"supported"}
        self.assertFalse(self.evaluate()["case_pass"])
    def test_wrong_source_or_hash_rejected(self):
        for field in ["source_id", "sha256", "text_sha256", "quote"]:
            with self.subTest(field=field):
                result = {**self.result,"citations":[{**self.citation,field:"forged"}]}
                self.assertFalse(self.evaluate(result)["case_pass"])
    def test_model_entities_not_published(self):
        self.result["entities"]=[{"name":"unsupported"}]
        self.assertFalse(self.evaluate()["case_pass"])
    def test_empty_nonabstaining_answer_fails(self):
        self.assertFalse(self.evaluate({"citations":[],"answer":"","evidence_only":True})["case_pass"])
    def test_abstention_only_passes_negative_case(self):
        result = {"abstained":True,"citations":[],"answer":"No observed outcomes are documented."}
        self.assertTrue(self.evaluate(result,True)["case_pass"])
        self.assertFalse(self.evaluate(result,False)["case_pass"])
    def test_scoring_never_infers_human_gold(self):
        self.assertEqual(self.evaluate()["human_historical_accuracy"],"NOT_SCORED_no_independent_human_QA_gold")

if __name__ == "__main__":
    unittest.main()
