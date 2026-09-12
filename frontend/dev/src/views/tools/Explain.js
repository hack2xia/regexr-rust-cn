/*
RegExr: Learn, Build, & Test RegEx
Copyright (C) 2017  gskinner.com, inc.

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details.

You should have received a copy of the GNU General Public License
along with this program.  If not, see <https://www.gnu.org/licenses/>.
*/

import $ from "../../utils/DOMUtils";
import Utils from "../../utils/Utils.js";

import ExpressionHighlighter from "../ExpressionHighlighter";

import app from "../../app";

export default class Explain {
	
	constructor(el) {
		this.el = el;
		$.addClass(el, "explain");
		this._update();
		
		this._bound_handleEvent = (evt) => this._handleEvent(evt);
		app.expression.addEventListener("change", this._bound_handleEvent);
		app.expression.highlighter.addEventListener("hover", this._bound_handleEvent);
	}
	
	
	cleanup() {
		$.empty(this.el);
		$.removeClass(this.el, "explain");
		app.expression.removeEventListener("change", this._bound_handleEvent);
		app.expression.highlighter.removeEventListener("hover", this._bound_handleEvent);
	}
	
// private methods:
	_update() {
		let el = $.empty(this.el), token = app.expression.token, expr = app.expression.value;
		this._divs = [];
		if (!token || token.next.type === "close") {
			el.innerHTML = "<span class='desc'>在上方输入的表达式会在此显示解析。</span>";
			return;
		}
		el.innerHTML = "<span class='desc'>滚动鼠标来高亮上方的正则表达式。通过点击来显示用法。</span>";
		while ((token = token.next) && (token.type !== "close")) {
			
			if (token.proxy || (token.open && token.open.proxy)) { continue; }
			
			let groupClasses = ExpressionHighlighter.GROUP_CLASS_BY_TYPE, pre = ExpressionHighlighter.CSS_PREFIX;
			let i = token.i, end = token.i+token.l, content=expr.substring(i, end).replace("<", "&lt;");
			if (token.set) {
				let set0=token.set[0], set2=token.set[2];
				content = "<span class='"+pre+(set0.clss || set0.type)+"'>"+expr.substring(set0.i, set0.i+set0.l)+"</span>";
				content += expr.substring(i, end);
				content += "<span class='"+pre+(set2.clss || set2.type)+"'>"+expr.substring(set2.i, set2.i+set2.l)+"</span>";
			}
			
			let className = pre + (token.clss || token.type);
			content = "<code class='token "+className+"'>"+content+"</code> ";
			if (!token.open) { content += app.reference.tipForToken(token); }
			else { content += "&nbsp;"; }
			let div = $.create("div", null, content, el);
			
			if (token.close) {
				className = groupClasses[token.clss || token.type];
				if (className) {
					className = className.replace("%depth%", Math.min(4,token.depth));
					$.addClass(div, className);
				}
				if (token.depth > 3) {
					div.innerHTML = "所以……你是想看看当嵌套分组时会发生什么对吧？ 其实吧，结果就是这样。"+
						" 我本来想表扬你对正则表达式玩笑的好奇心的， 但谷歌一下你就会发现：就凭几千万的网民是无法让正则表达式变得好玩的。"+
						" 除了大概你已经听说过的 “当你遇到一个问题，发现可以用正则表达式解决时，你遇到了两个问题” 之外，"+
						" 这不值得尝试，你觉得呢？";
					token = token.close.prv;
					this._divs.push(div);
					el = div;
					continue;
				}
				el = div;
			}
			
			div.token = token;
	
			if (token.open) {
				$.addClass(div, "close");
				div.proxy = el;
				el = el.parentNode;
			}
	
			if (token.error) {
				$.addClass(div, "error");
				if (token.error.warning) { $.addClass(div, "warning"); }
			}
	
			if (!token.open) {
				div.addEventListener("mouseover", this._handleMouseEvent);
				div.addEventListener("mouseout", this._handleMouseEvent);
				div.addEventListener("click", this._handleMouseEvent);
			}
			
			if (token.clss === "quant" || token.type === "lazy" || token.type === "possessive") {
				this._insertApplied(div);
			} else {
				this._divs.push(div);
			}
		}
	}

	_insertApplied(div) {
		let divs = this._divs, prv = div.token.prv, d, i=divs.length;
		while ((d = divs[--i]) && d.token !== prv) {} // search backwards for efficiency
		d = d.proxy||d;
		divs.splice(i, 0, div);
		d.insertAdjacentElement("afterend", div);
		$.addClass(div, "applied");
	}

	_handleHoverChange() {
		let token = app.expression.highlighter.hoverToken;
		$.removeClass($.queryAll("div.selected", this.el), "selected");
		$.removeClass($.queryAll("div.related", this.el), "related");
		if (!token) { return; }
		
		let div = this._findDiv(token);
		$.addClass(div, "selected");
		if (token.related) {
			for (let i = 0, l=token.related.length; i < l; i++) {
				$.addClass(this._findDiv(token.related[i]), "related");
			}
		}
	}
	
	_findDiv(token) {
		return Utils.find(this._divs, (div) => div.token === token);
	}

	_handleMouseEvent(evt) {
		let type = evt.type, token = evt.currentTarget.token;
		if (type == "click") { app.sidebar.showToken(token); }
		else { app.expression.highlighter.hoverToken = type === "mouseout" ? null : token; }
		evt.stopPropagation();
	}
	
	_handleEvent(evt) {
		if (evt.type === "change") { this._update(); }
		else if (evt.type === "hover") { this._handleHoverChange(); }
	}
}