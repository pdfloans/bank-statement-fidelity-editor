import sys
import os
import pytest

sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), '..')))

from pymupdf_pro_integration import _color_int_to_rgb, _classify_role, _missing_breakdown, _is_standard_14_basename

def test_color_int_to_rgb():
    assert _color_int_to_rgb(0xFF0000) == (1.0, 0.0, 0.0)
    assert _color_int_to_rgb(0x00FF00) == (0.0, 1.0, 0.0)
    assert _color_int_to_rgb(0x0000FF) == (0.0, 0.0, 1.0)
    assert _color_int_to_rgb(0x808080) == (128/255.0, 128/255.0, 128/255.0)

def test_classify_role():
    assert _classify_role({'1', '2', '3'}) == 'digits'
    assert _classify_role({'a', 'B', 'c'}) == 'letters'
    assert _classify_role({'1', 'a'}) == 'mixed'
    assert _classify_role({'.', ','}) == 'punctuation'
    assert _classify_role({' '}) == 'other'

def test_missing_breakdown():
    missing = ['1', 'a', '.', 'B', '2']
    breakdown = _missing_breakdown(missing)
    assert breakdown['digits'] == ['1', '2']
    assert breakdown['letters'] == ['a', 'B']
    assert breakdown['other'] == ['.']

def test_is_standard_14_basename():
    assert _is_standard_14_basename("Helvetica") == True
    assert _is_standard_14_basename("Times-Roman") == True
    assert _is_standard_14_basename("Courier-Bold") == True
    assert _is_standard_14_basename("ABCDEF+Helvetica") == True
    assert _is_standard_14_basename("ComicSans") == False
